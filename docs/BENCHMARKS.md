# Performance and memory benchmarks

This is a historical log. The portable `docs/benchmark-results-*.json` files it cites were removed before open-sourcing (they remain in git history); fresh results will replace them. Paths under `benchmark-results/` are raw reports the scripts write locally; that folder is gitignored, so it is not in a fresh clone.

## Shipped desktop apps: full startup-disk scans — 22 November 2026

**OpenDisk's App Store build scans the whole startup disk fastest, at 31.3 s, and rescans in 4.4 s from its cache. Hahakigami takes 41.2 s, which matches its native engine on the same disk (42.0–42.5 s). It gets no faster on a rescan (40.9 s), because the startup disk is always scanned in full. Radix takes 63–66 s. Hahakigami and Radix agree on the size of the disk. OpenDisk's total is about 170 GB higher, more than the APFS container actually holds.**

Unlike the earlier sections, these are the real GUI apps doing what a user does: open the app, scan **Macintosh HD**, wait for the result, then rescan in the same window. The person clicked Scan in each app.

| App (build) | First scan | Rescan | Total shown | Peak main-process RSS |
| --- | ---: | ---: | ---: | ---: |
| OpenDisk MAS 1.5 (55), Mac App Store | **31.3 s** | **4.4 s** | 916.66 GB | 4.2 GiB (second session, see below) |
| Hahakigami 0.1.0 at `b7ed74f` (development-signed `target/release` bundle) | 41.2 s | 40.9 s | 695.1 GiB = 746.4 GB | **1.42 GiB** |
| Radix 1.8.0 (14) | 63 s | 66 s | 746.34 GB | 5.7 GiB during scans, 6.8 GiB at most |

**Size check.** `diskutil apfs list` reports 801.9 GB in use across all volumes in the startup container, which includes VM, Preboot and Recovery. Hahakigami and Radix match each other to within 0.1 GB. OpenDisk's 916.66 GB is higher than everything the container holds. That fits the earlier finding that it counts every APFS clone copy at full size: in October it reported 914.1 GB, against Hahakigami's verified 785.6 GB.

**How it was measured.**
- The times are the ones each app showed, as reported by the person who ran the scans.
- For Hahakigami and Radix, a sampler also recorded the main process's CPU time and RSS every 100 ms (`ps`). Each scan was taken as the span where the process used at least half a CPU core. The sampled times match the app's own: Hahakigami 41.21 / 40.92 s, Radix 68.2 / 68.9 s. Radix's sampled spans are a few seconds longer than its own readout, which suggests it keeps working after the timer it shows stops. Each scan used about 143 s of CPU time in Hahakigami and about 415 s in Radix.
- **OpenDisk's memory comes from a second session.** In the first session the sampler stopped when OpenDisk's first process exited after a 10.8 s burst (54 s of CPU time, 909 MiB), so the timed scans were missed. A sampler that re-attaches on relaunch then recorded a second session of a scan and a rescan. The app runs as one process, with no XPC helper.
  - Within 7 s of launch, before any scan, RSS rose to 2.0–3.1 GiB at under one core of CPU. That fits OpenDisk loading its saved scan.
  - The full scan was a 35.4 s busy span (18:42:13). RSS climbed from 1.3 GiB to a **peak of 4.2 GiB**, and the process used about 1.1–1.75 cores.
  - Afterwards the process settled back to 1.4–2.3 GiB.
  - The person did not report the in-app times for this second session, so the times in the table are from the first session.
- Hahakigami's RSS covers only the Rust main process. Its WebKit helper processes are not counted.

**Caveats.**
- This is one run per app on one machine: Apple M4 Pro, 24 GiB RAM, macOS 26.5. Disk caches were warm from earlier activity, and the desktop was not controlled.
- Radix was still open, with about 2.2 GiB resident, while Hahakigami scanned. It used 0.01 s of CPU time in that window, so it competed only for memory.
- Each app ran with whatever Full Disk Access it already had. Nothing was granted or changed for this run.
- None of this was checked for path-level equivalence between the apps.

**What stands out for Hahakigami.**
- **The app adds almost no time to its engine. The disk now holds far more files.** Right after the app runs, the native engine (`target/release/examples/scan / 7`, the same 7 workers the app's Auto policy picks on this Mac) took **42.04 s and 42.48 s**. The app took 41.2 s. The October figure of about 26 s was for 4.76 million entries. Today the startup disk has **6.75 million entries in 1.00 million folders**, 42% more. The walk now takes 30.3–31.1 s, compared with about 20 s in October, roughly in proportion to the extra entries. Clone verification takes 10.0–10.3 s, compared with about 6.3 s. The engine's peak RSS is 1.25–1.38 GiB, close to the app's 1.42 GiB. Each run used about 7 s of user CPU and 134–138 s of system CPU, so the time goes almost entirely into filesystem calls in the kernel. Thermal and performance warnings: none recorded.
- **A rescan of `/` is always a full scan, by design.** Three separate rules force it:
  - `cache::eligible` refuses to save a result whose root is `/` (`src-tauri/src/cache.rs`).
  - `Baselines::acquire` skips event history for `/` (`src-tauri/src/baseline.rs`).
  - The engine's incremental path rejects it with "startup disk requires a full scan" (`crates/disk-scan/src/incremental.rs`).

  This applies to the result held in memory as well as the one on disk. Even without those rules, today's scan has 435 errors and is marked incomplete, and `eligible` also refuses any result with errors. The scan details panel (`ScanDetails.svelte`) shows "Full scan" in this case. The reason (`fallbackReason`) is not shown anywhere in the UI. OpenDisk's cached rescan is 9× faster, but its scans also overcount clones, and this comparison did not check how fresh its cached result is.
- **It uses the least memory.** Its peak of 1.42 GiB is about a third of OpenDisk's (4.2 GiB) and a quarter of Radix's (5.7 GiB).
- **Remaining gap to OpenDisk on a first scan: about 10 s.** That is roughly the time Hahakigami spends verifying clones, a step OpenDisk does not do. OpenDisk was not rerun on today's disk with its engine, so this split is an estimate.

## Results before clone verification: time to first result — 19 October 2026

**Prototype: the scanner now marks the end of the walk (`stats.walk_ms`) and publishes a progress event before clone verification starts. The app reports that stretch as a new `verifying` phase. Explorable results arrive 30–31% sooner than OpenDisk's on GitHub and Home, but still 2.4% later on startup `/`.**

| Scope, warm median | Hahakigami first result | Hahakigami verified sizes | OpenDisk final | First result vs. OpenDisk | Verified vs. OpenDisk |
| --- | ---: | ---: | ---: | ---: | ---: |
| GitHub folder (5 warm) | **3.140 s** | 3.874 s | 4.572 s | **−31.3%** | −15.3% |
| Home (3 warm) | **13.126 s** | 18.625 s | 18.793 s | **−30.2%** | −0.9% |
| Startup `/` (3 warm) | 18.583 s | 25.544 s | **18.140 s** | +2.4% | +40.8% |

**Definitions.**
- **First result** is the moment every entry is in the tree. Directory sizes are already summed up the tree as listings are committed, so from then on the explorer shows the complete tree with raw sizes. Raw sizes count each clone copy at full size, which is also how OpenDisk's final result counts them. The scan state stays `Running`, the progress panel says "Verifying shared clone sizes…", and cancel stays available.
- **Verified sizes** are final once the scan completes: shared clone blocks are deducted, and allocated totals fall to 180.4 / 614.5 / 785.6 GB, against OpenDisk's 203.4 / 658.0 / 914.1 GB.
- **Ranges:** startup ranges don't overlap (Hahakigami first result 18.533–18.638 s, OpenDisk 18.113–18.240 s), so the startup deficit is real, if small. GitHub and Home ranges don't overlap either, in Hahakigami's favour.

**Walk tuning tried for this prototype (rejected).**
- **Re-profile:** with the open gate in place, the sum of subtree walks is 17.61 s, against 18.71 s scanning `/` directly. That 1.1 s root-only overhead remains, entirely inside filesystem calls.
- **Vnode cache:** `kern.maxvnodes` is 263,168, far below either engine's 4.76M entries, so both thrash it equally. Changing it would be a system setting and is out of scope.
- **Skipping the per-child directory `fstatat`** except in firmlink-parent listings: no gain on `/` (walk 18.69 s versus 18.63–18.69 s), with 2 more errors. Rejected; nothing was implemented.
- **More workers with the open gate:** 9 workers walk 18.92–19.07 s and 11 workers 20.29–20.72 s, both slower than 7 (18.70–18.93 s). Clone verification also slows.

**Accuracy and limits.** Entries, errors, accounting checks and warning counts match the verified path; the only change is an earlier publication. Tests cover the published event on a real APFS clone fixture (state `Running`, all entries present, raw root size strictly above the verified size), cancellation (no walk-end publication), the app's phase mapping and the progress text. **Not measured:** how long the app takes to redraw at the walk-end event, and the incremental-rescan path. 28 fresh-process scans; no retries; inputs stable; 56 nominal power/thermal boundaries. Portable results (`benchmark-results-first-result-2026-10-19.json`, removed); runner and raw report in ignored `benchmark-results/first-result-2026-10-19/`.

## After bounded directory opens: `46585a1` versus Radix and OpenDisk — 19 October 2026

**The open gate holds up in the full comparison. Startup `/` is 4.68% faster than the pre-change cohort, and every other scope is flat to slightly faster. Hahakigami still leads OpenDisk on GitHub and Home and uses the least memory everywhere; OpenDisk still leads on startup, now by 41% instead of 48%.** Same runner, fixture and upstream binaries as the replication cohort below: Radix `b585f99` and OpenDisk `e421ee4`, whose upstream HEADs are unchanged.

| Core scan median (change vs. pre-change `5663014`) | Hahakigami `46585a1` | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | 40 ms (−4.8%) | **37.84 ms** | 411.49 ms |
| Local GitHub project folder | **3.888 s** (−0.9%) | 4.581 s | 6.168 s |
| Home | **18.434 s** (−0.7%) | 18.834 s | 25.908 s |
| Startup `/` | 25.738 s (**−4.7%**) | **18.237 s** | 39.151 s |

| Median high-water RSS | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | **20.17 MiB** | 22.72 MiB | 144.20 MiB |
| Local GitHub project folder | **145.72 MiB** | 224.50 MiB | 1,234.53 MiB |
| Home | **418.69 MiB** | 818.72 MiB | 3,353.42 MiB |
| Startup `/` | **596.69 MiB** | 951.42 MiB | 4,339.19 MiB |

- **Versus OpenDisk:** 15.13% faster on GitHub and 2.12% faster on Home. Slower by 41.13% on startup (pre-change 47.95%) and 5.71% on the fixture, where the gap is a 2 ms timer-truncation tie. Versus Radix: 28.9–90.3% faster on every scope.
- **Noise:** the Home ranges overlap (Hahakigami 18.281–19.173 s, OpenDisk 18.704–18.932 s), so Home is a narrow lead, not a decisive one. The startup ranges don't overlap the pre-change cohort's: 25.596–26.262 s now versus 26.901–27.027 s before.
- **Where startup stands:** walk medians are 18.753 s (pre-change 20.07 s) and the clone pass is 6.373 s (6.355 s). The remaining 7.5 s gap to OpenDisk is almost entirely the clone pass, which OpenDisk doesn't perform.
- **Coverage:** errors are unchanged (0 / 0 / 134 / 436). Clone query counts match live-tree drift (670,408 Home, 858,862 startup), and accounting checks complete on every run.
- **Run health:** 60/60 fresh-process scans succeeded with no retries. All 120 power/thermal boundaries were nominal, and the 90-second stall sampler never fired.

Portable results (`benchmark-results-gate-comparison-2026-10-19.json`, removed); raw report and log in ignored `benchmark-results/comparison-2026-10-19-gate/`.

## Bounded directory opens — 19 October 2026

**Retained: at most three scan workers may be inside a child-directory `openat` at once. `/System/Library` gets 36.6% faster and ties OpenDisk; startup `/` gets 4.75% faster; Home is unchanged within noise.** Listing, identity checks, commits and clone revalidation stay fully parallel.

| Core scan median | HEAD `5663014` | 3 permits (retained) | 4 permits | OpenDisk `e421ee4` |
| --- | ---: | ---: | ---: | ---: |
| `/System/Library` (7 workers) | 3.091 s | **1.960 s** (−36.59%) | 2.299 s (−25.62%) | 1.923 s |
| Home (6 workers) | 18.471 s | 18.359 s (−0.61%) | **18.322 s** (−0.81%) | 18.639 s |
| Startup `/` (7 workers) | 26.968 s | **25.687 s** (−4.75%) | 25.888 s (−4.00%) | **18.049 s** |

| Walk / clone pass median | HEAD | 3 permits | 4 permits |
| --- | ---: | ---: | ---: |
| `/System/Library` | 3.074 / 0.001 s | 1.945 / 0.001 s | 2.285 / 0.001 s |
| Home | 13.058 / 5.034 s | 12.902 / 5.076 s | 12.848 / 5.096 s |
| Startup `/` | 20.001 / 6.351 s | 18.702 / 6.355 s | 18.924 / 6.371 s |

**Why.** The per-directory profile (next section) showed `/System/Library` walking 62% slower than OpenDisk. It has about three entries per directory (146,868 directories for 436k entries). On that tree, fewer workers is faster: 4 workers take 2.0 s, 7 take 3.1 s and 14 take 8.2 s, while summed CPU time climbs from 5.9 to 98 s. A process sample at 14 workers put `__openat` at about 80% of samples, against 10% for `getattrlistbulk`, so the contention is inside the kernel lookup path. OpenDisk sidesteps it by running only three workers for non-root subtrees. Bounding just the `openat` call keeps all workers listing large directories. That is why large-directory trees such as `/Applications` and Home are unaffected.

**Rejected along the way.**
- **Rich attributes are not the cost:** requesting plain attributes on `/System/Library` made the walk slower (3.65–3.75 versus 3.22–3.29 s).
- **The pre-queue `fstatat` is minor:** skipping it saves about 0.13 s (4%), and it breaks firmlinked subtrees (21k entries lost).
- **The gate must not cover clone revalidation:** gating those reopens too slowed the clone pass from 6.5 s to 6.9–7.5 s.
- **Two permits:** slower on Home.

**Accuracy.** Each scope reports identical entry and error ranges for HEAD and both candidates. Clone query counts are identical (40 / 670,402 / 858,856), accounting checks complete on every run, and no run has warnings. On the deterministic 100,001-entry fixture, HEAD and the candidate report identical entries, directories, allocated bytes and clone queries. The opened-directory identity check is unchanged, so replacements still fail closed. The retained candidate's median RSS is within 2.4% of HEAD on every scope.

**Remaining startup gap.** OpenDisk is still 7.6 s faster on `/`. Of that, 6.4 s is the clone pass it doesn't perform, and about 0.65 s is walk time (18.70 versus 18.05 s total for OpenDisk).

**Method.** 56 successful fresh-process scans: one initial plus rotating warm rounds (five for `/System/Library`, three for Home and startup). No retries or discarded runs; binary fingerprints stable; all 112 power/thermal boundaries nominal with Low Power Mode off. The HEAD startup range includes one 30.920 s outlier; the candidate's startup range (25.434–25.711 s) does not overlap HEAD's other values (26.843–26.968 s). Same Apple M4 Pro, 14 threads, 24 GiB RAM, macOS 26.5. The limit of three is tuned on this host only; other hardware is unmeasured. Tests: 211 `disk-scan` tests (2 ignored), including two new gate tests, strict Clippy, workspace release check. Portable results (`benchmark-results-open-gate-2026-10-18.json`, removed); the runner and raw report are in ignored `benchmark-results/open-gate-2026-10-18/`.

## Where the startup gap lives: per-directory profile — 18 October 2026

**Hahakigami's walk is 6.2 s faster than OpenDisk on `/Users`, but 50–70% slower on the sealed System volume, and pays about 1.6 s extra when scanning from `/`. Most of the startup gap is still the 6.4 s clone pass.** Unchanged release binaries (Hahakigami `5663014` scanner, OpenDisk `e421ee4`) scanned each top-level startup subtree separately. Each scope ran in a fresh process: one initial and two rotating warm rounds per engine, 120 scans. Hahakigami used 7 workers; OpenDisk used `engine` defaults. All 240 power/thermal boundaries were nominal. **This is diagnostic.** Subtree scans do not reproduce startup-root mount and firmlink policy, and two warm rounds are not an acceptance cohort.

| Scope, warm median | Hahakigami total | Hahakigami walk | Clone pass | OpenDisk | Walk − OpenDisk | Entries (both) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `/Users` | 18.099 s | 12.592 s | 5.124 s | 18.777 s | **−6.185 s** | 3,165,049 |
| `/System/Library` | 3.081 s | 3.067 s | 0.001 s | 1.891 s | **+1.176 s** | 436,510 |
| `/private` | 2.056 s | 1.306 s | 0.678 s | 2.029 s | −0.723 s | 512,549 / 512,576 |
| `/Applications` | 1.208 s | 1.077 s | 0.117 s | 1.292 s | −0.215 s | 317,996 |
| `/Library` | 0.309 s | 0.308 s | 0 | 0.242 s | +0.066 s | 118,300 / 118,303 |
| `/opt` | 0.249 s | 0.249 s | 0 | 0.180 s | +0.069 s | 121,473 |
| `/System/iOSSupport`, `/System/Applications`, `/usr` | 0.307 s | 0.307 s | 0 | 0.190 s | +0.117 s | 87,253 |
| Preboot, Update and other small volumes/dirs | 0.051 s | 0.052 s | 0 | 0.052 s | ≈0 | 21,129 |
| **Sum of subtrees** | **25.359 s** | **18.955 s** | **5.919 s** | **24.652 s** | | |
| **`/` scanned directly** | **27.617 s** | **20.561 s** | **6.444 s** | **19.038 s** | **+1.523 s** | 4,758,296 / 4,779,658 |

Entry and error counts match per subtree (to within 27 live entries), so per-scope times compare like for like. The full `/` counts differ because of the known sibling-volume and mount policies.

**Findings.**
- **OpenDisk's `/` time is set by `/Users`.** Its subtrees add up to 24.65 s, yet `/` takes 19.04 s, about its `/Users` time. Alone, one subtree doesn't keep its workers busy; on `/` the rest of the tree fills the idle time.
- **Hahakigami is throughput-bound and gets no such overlap.** Its subtree walks sum to 18.96 s, and every worker is busy throughout (see the overlap section below). Scanning `/` directly costs **1.6 s more** than the sum of the parts. That root-only overhead is the first walk-side target: firmlink/Data-mirror handling, mount exclusion and alias prefiltering run only there.
- **The sealed System volume is the second target.** `/System/Library` takes 3.07 versus 1.89 s, about 142k versus 231k entries/s. Smaller System and read-only dirs show the same 50–70% gap. On data-volume trees (`/Users`, `/Applications`, `/private`), Hahakigami's walk is faster. Candidate causes, untested: the rich clone attributes (`file 0x607`, `ext 0x1300`) requested on a volume that cannot hold user clones, and per-child descriptor resolution on many small directories.
- **The clone pass dominates.** Even if both walk-side targets were removed (about 2.8 s), `/` would be about 17.8 s walk plus 6.4 s clone pass: still roughly 5 s behind OpenDisk. Beating OpenDisk on startup therefore also needs a cheaper clone pass, or results published before it finishes.

Raw rows, per-worker busy/metadata times and the runner are in ignored `benchmark-results/startup-per-dir-2026-10-18/`.

## Rejected before implementation: overlapping clone revalidation with enumeration — 18 October 2026

**Not implemented: the enumeration has no idle tail, and the filesystem path is already saturated, so overlapping the clone pass cannot shorten scans.** The idea was to revalidate completed parents' clone claims on idle enumeration workers, keeping full verification, and so hide the ~5 s Home and ~6.3 s startup clone pass.

- **No tail to fill.** In all six warm Home/startup runs of the replication cohort below, every worker is busy for 99% of the walk. Per-worker queue wait is 0–20 ms, and `tail_idle_us` is under 1 ms. Walk/finalization medians are about 13.1 / 5.4 s on Home and 20.1 / 6.96 s on startup. Clone revalidation alone takes 5.0 / 6.3 s wall time and about 30 / 44 worker-seconds.
- **No spare capacity for extra threads.** Single untimed-cohort Home probes with the unchanged release scanner:

| Home probe | Total | Walk | Clone revalidation |
| --- | ---: | ---: | ---: |
| One scan, 6 workers | 18.701 s | 13.231 s | 5.096 s |
| Two concurrent scans, 6 workers each | 37.179 / 38.155 s | 24.851 / 24.826 s | 11.905 / 12.934 s |
| One scan, 12 workers | 25.096 s | 15.006 s | 9.683 s |

Running two scans at once doubles both phases. Doubling the worker count makes both slower. Running revalidation alongside enumeration would therefore slow the walk by about as much as it hides. A gain would need less revalidation I/O, or publishing results before revalidation finishes. That would change when sizes are final, which is a product decision. These are single observations, not a medians cohort. Raw outputs are in ignored `benchmark-results/clone-overlap-feasibility-2026-10-18/`.

## Replication of the HEAD comparison — 18 October 2026

**The comparison below reproduces. Every ranking holds, and 11 of 12 core medians are within ±2% of the original cohort.** The same runner, binaries and fixture were re-run unchanged: Hahakigami `5663014` scanner, Radix `b585f99`, OpenDisk `e421ee4`. All 323 source/binary fingerprints and the fixture fingerprint match the original. Upstream HEADs were re-checked and are unchanged.

| Core scan median (change vs. original) | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | 42 ms (+7.7%) | **38.22 ms** (−0.1%) | 408.80 ms (−1.6%) |
| Local GitHub project folder | **3.925 s** (−0.5%) | 4.574 s (+0.4%) | 6.112 s (−0.8%) |
| Home | **18.566 s** (−0.0%) | 18.633 s (−0.3%) | 25.973 s (−0.0%) |
| Startup `/` | 27.003 s (+0.2%) | **18.252 s** (−1.9%) | 39.578 s (+1.1%) |

| Median high-water RSS | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | **19.92 MiB** | 23.42 MiB | 145.28 MiB |
| Local GitHub project folder | **146.09 MiB** | 226.97 MiB | 1,247.83 MiB |
| Home | **420.19 MiB** | 820.61 MiB | 3,924.78 MiB |
| Startup `/` | **585.47 MiB** | 947.94 MiB | 4,280.62 MiB |

- **Versus OpenDisk:** Hahakigami is 14.20% faster on GitHub (original: 13.36%) and 0.36% faster on Home (0.63%). It is 47.95% slower on startup (44.90%) and 9.88% slower on the fixture (1.94%). Versus Radix it is 28.5–89.7% faster on every scope. RSS remains lowest on every scope.
- **Fixture outlier:** Hahakigami's +3 ms fixture shift is the only change above 2%. Its warm values (39–44 ms) overlap the original's (38–41 ms), and its core timer truncates milliseconds. Treat the fixture as a tie with OpenDisk rather than a regression.
- **Noise:** the Home ranges still overlap (Hahakigami 18.353–18.577 s, OpenDisk 18.593–18.847 s), so Home is still a tie. The GitHub ranges still do not overlap. OpenDisk's Home RSS moved most (947.69 → 820.61 MiB).
- **Coverage:** errors are unchanged (0 / 0 on fixture and GitHub, 134 on Home, 436 / 438 / 434 on startup). Entry counts shifted slightly on the live trees; on the fixture they are identical.
- **Run health:** 60/60 fresh-process scans succeeded with no retries. All 120 power/thermal boundaries were nominal, with Low Power Mode off. A 90-second stall sampler never fired; the slowest Hahakigami Home scan was the 34.1 s initial run. The original's 900 s stall remains unreproduced.

Portable medians and per-cell changes (`benchmark-results-head-replication-2026-10-18.json`, removed). The raw report and log are in ignored `benchmark-results/comparison-2026-10-18-replicate/`.

## HEAD versus current Radix and OpenDisk — 18 October 2026

**Hahakigami `5663014` leads on the GitHub project folder, essentially ties OpenDisk on Home, and has the lowest scanner memory on every scope. OpenDisk keeps a 45% startup `/` lead.** This cohort compares the committed clone-revalidation optimization with the current public upstream HEADs: [Radix `b585f99`](https://github.com/colinvkim/Radix/commit/b585f99eb457b48f8f26322becf8f3d9b73ebefd) (unchanged) and [OpenDisk `e421ee4`](https://github.com/137137137/OpenDisk/commit/e421ee421cb609ba5b0dea8a3018a3965891cb8e). OpenDisk's two new commits only touch `DeviceMonitor.swift` and the project version. Neither file is compiled into the scanner adapter. This is a **scanner-engine comparison** using default policies and no persistent scan-cache reuse.

| Workload / reported core scan median | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture, 100,001 entries | 39 ms | **38.26 ms** | 415.49 ms |
| Local GitHub project folder | **3.946 s** | 4.554 s | 6.163 s |
| Home | **18.568 s** | 18.686 s | 25.985 s |
| Startup `/` | 26.950 s | **18.600 s** | 39.167 s |

| Workload / median process high-water RSS | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | **19.77 MiB** | 22.88 MiB | 143.81 MiB |
| Local GitHub project folder | **145.55 MiB** | 220.81 MiB | 1,245.70 MiB |
| Home | **423.31 MiB** | 947.69 MiB | 3,693.33 MiB |
| Startup `/` | **599.39 MiB** | 879.70 MiB | 4,288.03 MiB |

Relative to OpenDisk, Hahakigami's core median is **13.36% lower on GitHub** and **0.63% lower on Home**. It is **1.94% higher on the fixture** and **44.90% higher on startup**. Its RSS is **13.59 / 34.09 / 55.33 / 31.86% lower** in fixture/GitHub/Home/startup order. Relative to Radix, Hahakigami is **28.5–90.6% faster** and uses **86.0–88.5% less RSS** on every scope. The Home result is within run-to-run noise: warm ranges overlap (Hahakigami 18.448–18.579 s, OpenDisk 18.548–18.756 s). The GitHub ranges do not overlap. These are default-policy observations on one host, not identical-work guarantees.

Whole-process wall medians are **43 / 48 / 460 ms** (fixture), **3.952 / 4.593 / 6.669 s** (GitHub), **18.585 / 18.811 / 28.212 s** (Home) and **26.970 / 18.762 / 41.987 s** (startup), in Hahakigami/OpenDisk/Radix order. On wall time Hahakigami is 10.6% faster than OpenDisk on the fixture, although OpenDisk's reported core median is lower. Core timer boundaries differ, and Hahakigami truncates milliseconds.

**Coverage and accounting.**
- **Fixture and GitHub:** all scans report zero errors. Hahakigami and OpenDisk each retain 100,001 and 910,013 entries. Radix retains 66,674 and 761,701 entries, with 1 and 786 summarized directories.
- **Allocated bytes:** on GitHub, Hahakigami and Radix both report 180,109,848,576 bytes and OpenDisk 203,153,686,528. On the fixture the totals are 8,192 / 12,288 / 8,192 bytes, consistent with OpenDisk's lack of full-clone deduplication.
- **Home:** every engine reports 134 errors. Entries are 3,164,874–3,164,877 for Hahakigami, 3,164,876–3,164,877 for OpenDisk and 2,915,746–2,915,748 for Radix (2,718 summarized directories).
- **Startup:** errors are 436 / 438 / 434. Hahakigami records 11 exclusions. Entries are 4,759,506–4,760,027 for Hahakigami, 4,780,129–4,780,672 for OpenDisk and 3,737,725–3,738,244 for Radix (7,381 summarized directories). OpenDisk composes sibling system volumes, and Hahakigami excludes unrelated mounts.
- **Equivalence:** exact path-set and accounting equivalence were not re-verified for this cohort.

**Method and verification.**
- **Schedule:** one separate initial observation per engine and scope, plus cyclically rotating warm rounds: five for the fixture and GitHub, three for Home and startup.
- **Totals:** **60 successful fresh-process full scans**, with no retries, discarded observations, cache purges or builds during timing.
- **Stability:** source/binary fingerprints and the fixture fingerprint stayed unchanged. All **120 power/thermal boundaries** were nominal with Low Power Mode off.
- **Host and workers:** Apple M4 Pro, 14 threads, 24 GiB RAM, macOS 26.5. Hahakigami uses six workers for the fixture, GitHub and Home, and seven for startup. OpenDisk and Radix keep their defaults.
- **Not measured:** desktop UI, launch, incremental rescans and total app memory.

**Discarded first attempt: unexplained Home stall.** The same binaries were first run with a 900-second timeout. The fixture and GitHub scopes completed, but the initial Hahakigami Home scan **timed out at 900 s and produced no output**. OpenDisk's following Home scan took 158.3 s instead of its usual ~18.7 s; Radix's took a normal 26.6 s. No stack sample was captured. The run was stopped and discarded.

Six direct Hahakigami Home scans immediately afterwards completed in 18.39–18.59 s. The replacement cohort ran with a 300-second timeout and a 90-second stall sampler, which never fired. **The cause is unknown and the stall remains unreproduced.** It is not counted in the medians above, but it should not be read as evidence that the scanner cannot stall.

Portable observations, comparison ratios, build commands and the aborted attempt's Home rows (`benchmark-results-head-comparison-2026-10-18.json`, removed). The runner, upstream build script, adapters and both raw reports are in ignored `benchmark-results/comparison-2026-10-18-head/`. Original raw directories: `/private/tmp/hahakigami-head-compare-ypWi/` (retained cohort) and `/private/tmp/hahakigami-head-compare-p4NX/` (discarded attempt).

## Clone revalidation optimization — 18 October 2026

**Retained fresh-cursor fusion and a bounded ancestor descriptor cache. Clone revalidation medians fall 6.38% Home / 7.13% startup, and full scans improve 0.85% / 3.19%, with effectively unchanged median scanner memory.** The baseline is the previous retained locality candidate, frozen at the start of this task, not bare Git HEAD. Source manifests and a task-relative patch identify the change.

| Full scan, reported core median | Previous candidate | Optimized | OpenDisk `368eef2` |
| --- | ---: | ---: | ---: |
| Home | 18.728 s | **18.568 s** | 18.755 s |
| Startup `/` | 27.306 s | **26.435 s** | 17.853 s |
| Home clone revalidation | 5.384 s | **5.040 s** | No equivalent clone pass |
| Startup clone revalidation | 6.694 s | **6.217 s** | No equivalent clone pass |

| Median scanner-process high-water RSS | Previous candidate | Optimized | OpenDisk |
| --- | ---: | ---: | ---: |
| Home | 419.22 MiB | **419.75 MiB** | 865.19 MiB |
| Startup `/` | 590.44 MiB | **586.95 MiB** | 970.25 MiB |

The candidate wins all six same-round warm timing pairs. Median RSS changes **+0.13% Home / −0.59% startup**, preserving **51.5% / 39.5% lower median RSS than OpenDisk** in this cohort. These small baseline-relative differences do not establish an algorithmic memory reduction. The candidate narrowly leads OpenDisk on Home here; OpenDisk retains a substantial startup lead. Do not combine absolute timings from different cohorts: live data, cache and host variation affect the measurements.

### Retained implementation and accuracy

Bulk validation now obtains a fresh target-directory cursor during parent preparation and reads it directly, avoiding a second reopen of the same directory. Root and deep-path fallbacks still acquire independent cursors; descriptor identity is checked again after the preparation hook. Exhausted cursors are never reused as fresh listings.

The existing per-worker parent cache retains a small pool of retired strict ancestors across branch changes. Reuse requires both a fresh namespace-relative identity check and a matching held descriptor. Final target directories are opened afresh, replacements fail closed, and the combined active-chain/spare pool keeps the existing **32-descriptor limit per worker**. Both vectors and their names contribute to scratch accounting. No new per-entry fields, larger worker pool, relaxed access flags or changed task ordering are retained.

Member metadata checks, complete-stream and duplicate handling, unsupported fallback, whole-group rejection, owner selection and cancellation remain in place. All eight baseline/candidate observations per scope have identical query counts and deductions: **670,407 / 858,841 member queries**, **85,538 / 111,432 parent walks**, and **43,471,556,608 / 94,833,057,792 deducted bytes** for Home/startup. Bulk/direct query counts and worker counts also match. This improvement removes directory-opening work rather than skipping clone validation.

A separate same-snapshot diagnostic compares legacy, cursor-only, branch-cache-only and combined strategies. All **20 exact status vectors match**, with accounting checks complete. Combined warm median validation is **5.849 s versus 6.248 s** legacy (6.39% lower). Structural preorder scheduling was rejected: **6.554 s** versus **6.194 s** legacy in its diagnostic. Smaller worker pools showed no benefit. Ancestor access-mode changes were researched but not implemented. Diagnostic timing and cumulative process RSS are excluded from full-scan acceptance.

### Verification and limits

The final cohort contains **24 successful measurement processes**: a separate initial observation and three rotating warm rounds per engine/scope, fresh processes, unchanged six/seven-worker policy, no retries, discarded observations, cache purges or builds during timing. Initial observations are not cold-cache measurements. Frozen fingerprints match; all 48 power/thermal boundaries report nominal state with Low Power Mode off. Local retention guards pass: lower full-scan median, at least 3% lower clone-pass median, no more than 3% median RSS increase, and lower median RSS than OpenDisk, separately on both scopes.

**401 workspace tests pass, 22 ignored; 54 focused clone tests pass**, plus formatting, strict scanner/all-target Clippy and both release builds. The workspace suite preceded a semantically identical permission-test literal correction (`0` to `0o0`); focused tests and Clippy ran afterward. Tests cover cursor exhaustion, roots, fallback depth, branch reuse/caps, replacements, renames and owned-directory permission revocation. Independent Max review approves descriptor lifetime, validation safety and final evidence with no unresolved findings; its verdict is recorded with the raw artifacts.

Four untimed exports show exact **100,001-entry fixture** equivalence and equal clone deductions. The **909,187-entry GitHub** exports have identical paths, identities and kinds, with equal **23,043,837,952-byte deductions**. Eleven build artifacts differ in allocated size between sequential scans, so strict live GitHub metadata equivalence does not pass; that discrepancy is preserved without a retry.

Accounting checks complete without warnings on every Hahakigami run. Full-scope coverage remains incomplete: **134 Home errors** for both engines; **435 startup errors and 11 exclusions** for Hahakigami versus **437 errors** for OpenDisk. Startup entry counts are approximately **4.759 million versus 4.779 million**; mount/accounting policies differ, and exact full-volume path equivalence is unverified. Whole-process wall medians also improve: **18.741 → 18.583 s Home / 27.326 → 26.453 s startup**; OpenDisk is **18.874 / 18.021 s**. Core timer boundaries differ. These are native-engine, warm-cache measurements on one M4 Pro, not GUI memory or a general hardware guarantee. Privacy grants and normal app sessions are unchanged.

Portable raw observations, source hashes, task-relative patch, export comparisons, diagnostic and check summaries: clone-pass results (`benchmark-results-clone-pass-2026-10-18.json`, removed). Reproduction scripts, frozen sources and logs are retained locally under `benchmark-results/clone-pass-2026-10-18/`; large binaries and NDJSON remain at inventoried temporary paths.

## Full-scan locality optimization — 18 October 2026

**Retained two bounded changes: observed full-scan medians improve 2.31% on Home and 2.09% on startup, with essentially unchanged median scanner RSS. OpenDisk remains faster.** Results compare the unchanged `917c2ae` scanner and the candidate in the same final cohort; they do not infer progress by subtracting results from earlier cohorts.

| Full scan, reported core median | Baseline | Optimized | OpenDisk `368eef2` |
| --- | ---: | ---: | ---: |
| Home | 19.775 s | **19.319 s** | 18.724 s |
| Startup `/` | 27.673 s | **27.096 s** | 17.976 s |

| Median scanner-process high-water RSS | Baseline | Optimized | OpenDisk |
| --- | ---: | ---: | ---: |
| Home | 488.27 MiB | **487.83 MiB** | 2,048.86 MiB |
| Startup `/` | 590.52 MiB | **585.89 MiB** | 955.91 MiB |

The candidate wins all **six same-round warm timing pairs**. Median RSS changes **−0.09% Home / −0.78% startup**; these small changes are not evidence of an algorithmic RAM reduction. Some paired RSS observations increase, by up to **1.97%**. Candidate warm RSS ranges are **411.25–490.38 MiB Home** and **568.84–598.00 MiB startup**. OpenDisk varies substantially: **934.14–2,076.95 MiB Home**, **779.38–1,066.00 MiB startup**. The candidate stays below OpenDisk throughout this local warm cohort. The Home OpenDisk median is particularly sensitive to this three-observation sample; it is not a general memory guarantee.

### Where the full-scan time goes

The baseline's measured enumeration medians are **13.468 s Home / 20.054 s startup**, followed by **6.289 / 7.590 s finalization**. Clone revalidation alone takes **5.880 / 6.975 s**. Per-worker diagnostics put approximately 97–98% of busy processing wall time inside metadata/enumeration; those summed intervals are not CPU utilization or additive scan latency. An owned-process sample also places filesystem calls (`getattrlistbulk`, `openat`, `fstatat`) at the top of the stacks. Profiled timing is excluded from acceptance.

Two source-level work differences matter. [OpenDisk's reader](https://github.com/137137137/OpenDisk/blob/368eef266b6fe31df73735e4df015f9a7f1a1757/OpenDisk/Services/Scanning/SystemInterop/BulkDirectoryReader.swift) requests allocation/link-count metadata and validates each opened directory. Hahakigami additionally resolves each directory/symlink child relative to the held descriptor before queueing, preserving actual mount/firmlink identities and replacement protection. Hahakigami also revalidates full-clone identity, link counts and storage metadata before shared-storage deductions; OpenDisk does not perform that clone-accounting pass. These observations explain substantial additional work, but do not isolate the exact causal share of the whole inter-engine gap: scope, mount and accounting policies differ.

### Retained changes

- **Firmlink parent prefilter:** derive a small set of immediate parent paths from already verified aliases, then check that set once per listing. Full file paths and exact alias lookups are needed only in relevant listings; directories still receive their traversal/scope paths. Verified file aliases, directory aliases, native bytes and identity replacement checks retain their behavior.
- **Clone parent locality:** keep expensive whole-parent tasks first, then order small tasks by directory ID before the original claim-range tie-break. Sibling directories receive nearby IDs, helping the existing bounded ancestry cache reuse handles. Every task retains its original disjoint status range, fresh root/ancestor checks, member probes, whole-group rejection, stable owner selection and cancellation handling. The extra temporary task field is included automatically in `size_of::<ParentTask>()` scratch accounting; retained node layouts and serialized formats are unchanged.

Matched candidate revalidation medians are **5.444 s Home / 6.640 s startup**. All warm baseline/candidate pairs retain exactly **670,407 / 858,717 member-validation queries**, **85,538 / 111,370 parent walks**, and equal bulk/direct query counts and worker counts for Home/startup respectively. Clone deductions match exactly: **43,471,482,880 / 94,797,033,472 bytes**. No validation-query reduction is credited as a speed optimization. Enumeration medians change to **13.331 / 19.795 s**; phase variation is retained and does not establish precise attribution to one instruction.

An earlier isolated root cohort gave **28.275 s baseline / 27.446 s parent-ID ordering / 28.050 s firmlink filtering**, each with three warm rounds. A more complex structural-ancestry ordering was rejected after an exploratory pair (**28.127 → 28.534 s**, finalization **7.513 → 8.350 s**). That one pair is a candidate-selection observation, not proof of general regression. Its code was not retained.

### Verification and limits

The final cohort has **24 successful measurement processes**: one separate initial scan and three cyclically rotating warm rounds per variant/scope, fresh processes/full scans, unchanged worker policy (six Home/seven startup), and no retries, dropped observations, cache purges or builds during timing. Source/binary fingerprints remain unchanged; all **48 power/thermal boundaries** are nominal with Low Power Mode off. Same Apple M4 Pro, 14 threads, 24 GiB RAM, macOS 26.5. Local retention guards pass: at least 1% lower median time, no more than 3% median RSS increase over baseline, and lower median RSS than OpenDisk, separately on both scopes.

All **393 workspace tests pass, 22 ignored**, including **199 scanner tests, 2 ignored**. Formatting and strict scanner/all-target Clippy pass. Tests cover exact alias parents and identities, native bytes, original status slots, APFS serial/parallel ownership, ancestor replacement, cancellation and bounded descriptor budgets. Independent High source review and final evidence review approve the retained candidate without findings.

Four separate untimed exports verify the 100,001-entry controlled fixture with **exact path/identity/kind/raw-size equivalence and equal accounting totals**. The **906,886-entry GitHub exports** have identical paths, identities and kinds, with equal **23,043,764,224-byte clone deductions**. Three build artifacts report different allocated sizes between scans, so strict GitHub metadata equivalence does **not** pass; the discrepancy is retained rather than retried away.

Full Home/startup coverage remains incomplete under inherited permissions: all engines report **134 Home errors**; Hahakigami reports **435 startup errors and 11 exclusions**, OpenDisk **437 startup errors**. Home contains about 3.161 million retained entries; startup contains about 4.753 million for Hahakigami versus 4.774 million for OpenDisk. Live files and mount policies differ, and full-scope exact path equivalence is unverified. Accounting checks complete without warnings on every Hahakigami observation. No privacy grants or normal app sessions are changed.

Whole-process wall medians preserve the ranking: Home **19.794 → 19.334 s**, OpenDisk **18.854 s**; startup **27.694 → 27.116 s**, OpenDisk **18.153 s**. Core timer boundaries differ and Hahakigami truncates milliseconds. Memory is native-process high-water, not physical footprint or total GUI memory. Three warm rounds on one host establish these local observations, not all-hardware or full-app acceptance.

Raw observations, guards, source/build hashes, profiles and coverage discrepancies (`benchmark-results-fullscan-optimization-2026-10-18.json`, removed). Ignored permanent source archives, candidate patch, review receipt, tool archive and profile are under `benchmark-results/fullscan-optimization-2026-10-18/`; large exports, frozen binaries and original logs are inventoried under `/private/tmp/hahakigami-fullscan-opt-20261018-i3zf38n0/`.


## Full Home and startup scans — 18 October 2026

**OpenDisk is faster on both full requested scopes; Hahakigami uses less peak scanner-process memory.** This extends the preceding fixture/GitHub comparison using the same source revisions and unchanged release scanner binaries: Hahakigami `917c2ae`, Radix `b585f99`, OpenDisk `368eef2`. It measures full scanner-engine traversals of Home and startup `/`, with fresh processes and no persistent scan-cache reuse. It does not measure GUI rendering, interactions or total desktop-app RAM.

| Reported core scan median | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Home | 19.792 s | **18.403 s** | 26.237 s |
| Startup `/` | 28.132 s | **18.286 s** | 39.474 s |

| Median process high-water RSS | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Home | **409.97 MiB** | 855.03 MiB | 3,415.83 MiB |
| Startup `/` | **589.38 MiB** | 929.28 MiB | 4,256.45 MiB |

Against OpenDisk, Hahakigami takes **7.55% longer on Home and 53.85% longer on startup**, with **52.05% and 36.58% lower peak RSS**, respectively. Against Radix it takes **24.57% and 28.73% less time**, with **88.00% and 86.15% lower peak RSS**. These are observations under each scanner's default coverage and accounting policies, not an identical-work speed certification. The larger GitHub-folder win does not generalize to these full scopes.

**Coverage limitations are material.** All engines report errors: **134 per Home scan**; startup reports **433 for Hahakigami, 435–436 for OpenDisk, and 431 for Radix**. Hahakigami records zero Home exclusions and **11 startup exclusions**, and completes clone-accounting checks without warnings on all observations. No OS permissions or privacy grants were changed. Radix's own completion flag remains true despite reported warnings; this report conservatively treats every observation as having incomplete error-free coverage.

Across initial and warm observations, Hahakigami/OpenDisk retain about **3.151 million Home entries** and Radix about **2.902 million**. On startup they retain approximately **4.740 / 4.761 / 3.718–3.725 million entries**, respectively. Radix summarizes **2,713 Home directories** and **7,361–7,376 startup directories**. OpenDisk's startup engine composes additional sibling system volumes; Hahakigami excludes unrelated mounts and avoids mirrored Data traversal. The live filesystem changes between observations, and allocation policies differ. **Exact path-set and accounting equivalence are unverified for this cohort**; equal Home error counts do not prove equal access.

**Method and verification.** One separate initial observation plus **three cyclically rotating warm rounds** per engine/scope gives **24 successful measurement processes**. Each engine occupies every order position once in the warm rounds. No timing observation is retried or discarded, caches are not purged, and no builds run during timing. Initial scans are not cold-cache claims. Scanner/binary fingerprints match the earlier comparison and remain stable through this cohort. All **48 process-boundary observations** report nominal thermal state and Low Power Mode off. Same Apple M4 Pro, 14 threads, 24 GiB RAM, macOS 26.5. Hahakigami uses its nominal Auto bounds of six workers for Home and seven for startup; other engines retain their defaults.

Core timer boundaries remain different and Hahakigami truncates milliseconds. Whole-process wall medians are **19.811 / 18.521 / 28.528 s** for Home and **28.151 / 18.445 / 42.524 s** for startup, in Hahakigami/OpenDisk/Radix order, preserving the ranking. Memory is macOS `getrusage` process high-water, not physical footprint or a complete GUI app's memory. Three warm observations on one host provide a small local cohort, not statistical evidence across hardware.

Raw observations, coverage/error ranges, source/build hashes and comparison calculations (`benchmark-results-full-scans-2026-10-18.json`, removed). Ignored runner and tool archive: `benchmark-results/comparison-2026-10-18/run_full_scans.py` and `full-scan-tools.tar.gz`; original raw report/log: `/private/tmp/hahakigami-full-scans-20261018-e96n5slm/`. The regular app session and production implementation remain unchanged.


## Current upstream comparison — 18 October 2026

**Hahakigami leads on the larger project workload; OpenDisk leads on the small fixture's reported core scan time.** Fresh release builds compare Hahakigami `917c2ae` with the current public upstream HEADs checked for this run: [Radix `b585f99`](https://github.com/colinvkim/Radix/commit/b585f99eb457b48f8f26322becf8f3d9b73ebefd) and [OpenDisk `368eef2`](https://github.com/137137137/OpenDisk/commit/368eef266b6fe31df73735e4df015f9a7f1a1757). This is a **scanner-engine comparison**, with normal engine policies and no persistent scan-cache reuse. It does not measure desktop UI, launch, interactions, incremental rescans or total application RAM.

One Apple M4 Pro host, 14 CPU threads, 24 GiB RAM, macOS 26.5. Each engine receives one separate initial observation and **nine warm rounds**, with cyclic rotation giving each engine three turns in each order position. Every observation uses a fresh process and full scan. No cache purge, retries, discarded observations or build work occurs during timing. Initial observations are not cold-cache claims. Hahakigami uses six workers, matching its nominal Auto folder policy on this host; OpenDisk and Radix retain their defaults, reporting configured limits of three and seven respectively. Upstream traversal sources are unmodified; separate adapters collect results. OpenDisk's existing benchmark cache shim returns no cached scan and performs no cache writes.

| Workload / reported core scan median | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture, 100,001 root-inclusive entries | 49.0 ms | **42.53 ms** | 430.15 ms |
| Local GitHub project folder | **4.192 s** | 4.703 s | 6.269 s |

| Workload / median process high-water RSS | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Controlled fixture | **21.75 MiB** | 23.78 MiB | 146.38 MiB |
| Local GitHub project folder | **143.86 MiB** | 230.41 MiB | 1,237.06 MiB |

On the project folder, Hahakigami's reported scan median is **10.87% lower than OpenDisk** and **33.14% lower than Radix**, with **37.56% and 88.37% lower scanner-process peak RSS**, respectively. On the fixture, its core median is **15.22% higher than OpenDisk**, with **8.54% lower RSS**. These are observed default-policy results on one host, not a universal or identical-work performance guarantee. Warm samples and outliers remain in the raw report.

**Timer boundaries matter.** Hahakigami truncates core milliseconds after root initialization; the Swift adapters measure engine initialization through completion/rollup return. Separately recorded whole-process wall medians, including startup, result serialization and teardown, are **53.11 / 52.72 / 476.11 ms** for the fixture and **4.197 / 4.752 / 6.814 s** for the project folder, in Hahakigami/OpenDisk/Radix order. Thus the fixture's whole-process Hahakigami/OpenDisk latency is nearly tied even though the reported core medians differ. RSS is `getrusage` process high-water on macOS, not physical footprint or a complete GUI application memory measurement.

**Coverage and accounting.** Hahakigami and OpenDisk each retain **100,001 fixture entries and 898,379 project entries**, with identical exported relative-path sets in both scopes. Radix retains **66,674 and 750,095 entries**, with **one and 782 summarized directories**, respectively. On the fixture, comparison finds 33,328 Hahakigami-only paths and one Radix-only summary path; on the project folder it finds 148,284 Hahakigami-only paths. Fewer retained nodes are a default summarization behavior, not evidence of scan failure.

The fixture includes a hard link, APFS clone, sparse file, symlinks, Unicode/newline names, and deep/wide listings. Hahakigami and Radix each attribute **8,192 allocated bytes**, while OpenDisk reports **12,288 bytes**, consistent with its lack of full-clone deduplication. On the project folder the respective totals are **177,952,935,936 / 200,945,246,208 / 177,952,935,936 bytes**. Exported per-node metadata contracts also differ: OpenDisk does not retain symlink kinds, and directory own/aggregate size conventions differ. The strict coverage comparator correctly declines full metadata/policy equivalence; equal path sets do not establish equal accounting work.

**Verification:** all **60 timed scans succeed with zero reported errors**; Hahakigami accounting checks complete, without exclusions. All source/binary fingerprints and fixture metadata remain stable across timing. All **120 process-boundary observations** report nominal thermal state and Low Power Mode off. Six separate completed coverage exports verify the returned path sets; the project scope is live and these are post-timing observations rather than a filesystem snapshot. The regular app session is preserved.

Raw observations, medians, source/build hashes, commands and coverage comparisons (`benchmark-results-current-upstream-2026-10-18.json`, removed). Ignored reproduction scripts, upstream checkouts, build logs and a benchmark-tool archive are under `benchmark-results/comparison-2026-10-18/`; large exports and raw logs are inventoried in the JSON under `/private/tmp/hahakigami-comparison-20261018-uba3js_7/`. No production implementation changes were made for this comparison.


## Decoded-record buffer reuse — 18 October 2026

**Retained listing-local decoded storage reuse; the OpenDisk clear-win target remains unmet.** One shared checked parser emits either the existing borrowed-name records or records containing validated absolute byte ranges. Directory enumeration clears and reuses the latter vector across batches, reserving only when a batch exceeds its capacity. It drops scratch before fallback, terminal errors and tree publication. Full-batch validation, native-name checks, metadata/descriptor checks, error order, calibration limits, clone ordinals and cancellation remain intact. Both decoded forms stay 96 bytes; retained 80-byte records, 64-byte entry results and serialized formats remain unchanged. No unsafe code, persistent worker scratch, metadata fields/flags, packed-buffer sizes, entry growth policy or concurrency changes were introduced.

| 100,001-entry fixture, native scanner | Retained control | Decoded-buffer reuse | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 132.0 ms | 131.0 ms | **130.765 ms** |
| Median process high-water RSS | 19.484 MiB | 19.742 MiB | 23.672 MiB |

The candidate improves the matched native median by **1 ms**, but remains **0.235 ms above OpenDisk** and wins only **9/30 paired rounds**. It fails the unchanged speed gate requiring a median at least 1 ms below OpenDisk and at least 18 wins. Fixture native RSS rises **1.32%** against control, within the 1.03 guard, and remains **16.60% below OpenDisk**. This is an observed within-cohort gain, not proof of a general speed improvement or of the precise size of progress across previous cohorts.

**Reservations and phase profiles.** Thirty diagnostic observations compare control, fresh offset records and reused offset records: one initial and nine balanced rotating warm rounds per variant. The fresh-offset control differs from reuse only by resetting the decoded vector before each read. All processes decode 100,000 descendants in 60 nonempty batches, with 72 successful reads/parses including EOF. Observed reservation/capacity-growth events fall **60 → 15**, with **45 reused batches**. Summed requested and observed growth payload capacities fall **9,600,000 → 787,392 bytes**. These are API requests and vector capacities, not malloc calls, bytes copied, physical allocation or process RAM. The largest listing capacity remains 2,184 records; the sum of exit capacities across listings is not simultaneous memory.

The measured listing-processing interval does **not** improve: control **2.902 ms**, fresh offsets **2.869 ms**, reuse **2.954 ms**. This new diagnostic measures whole listing-attempt elapsed time, including setup and decoded-scratch destruction, minus reader intervals. A scalar guard is declared before scratch and emits after scratch destruction, with no per-entry clocks or allocator hooks. It is not CPU time, concurrent directory sums do not add to scan latency, and its boundaries differ from earlier narrow bulk-phase profiles. Instrumented timings are excluded from acceptance. No claim attributes the full observed 1 ms native difference to a measured materialization improvement.

**Regression guards and RAM cost.** On the **896,763-entry GitHub scope**, native medians change **3.963 → 3.948 s** and app medians **4.030 → 4.020 s**. Fixture app medians change **155.905 → 155.561 ms**. All timing guards pass at the 1.05 limit. Larger native RSS increases **145.469 → 149.781 MiB (+2.9646%)**, narrowly passing the 3% guard with about **54 KB of median threshold slack**. This limited margin is a material constraint, not a wider memory guarantee. App-process RSS is mixed: fixture **106.375 → 107.266 MiB**, GitHub **239.156 → 237.188 MiB**. Fewer reservation requests do not establish a RAM reduction; the native RSS increases are preserved in the result.

**Accounting and verification.** All **147 timed observations** succeed: 93 fixture/native cases with 30 balanced warm rounds, followed by 54 app/larger cases with five rotating warm rounds per cell, each with a separate initial observation. All **762 native-stage and 762 remaining-stage inputs** stay unchanged, and all **294 power/thermal boundaries** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings, errors or exclusions, with equal totals across versions/layers. Fixture totals remain 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte clone deduction; OpenDisk reports 12,288 allocated bytes under its default policy. Eight complete native exports and eight exact path/identity/kind/raw-size comparisons pass before/after timing, with all **758 export inputs** stable across phases. Ordering may differ in raw NDJSON; path-keyed equivalence passes. No timed observation was retried or discarded.

The candidate passes **397 Rust tests, 22 ignored**, with the benchmark feature, plus formatting and strict workspace/all-target Clippy with default and benchmark features. A frozen original byte parser and original enumeration provide independent checks for masks, truncations, corruptions, all native bytes, name pointers, errors, accounting fields, limits, restarts and cancellation. Reuse tests overwrite/replace/grow buffers and verify shrinking/error/EOF clearing and capacity stability. Fresh isolated baseline and candidate apps both pass frontend checking, production compilation, development-signature verification and candidate smoke. High source and evidence reviews approve correctness and retention within the declared guards. All 376 workspace inputs matched the frozen reuse source before documentation; all 105 frontend inputs and the normal app session are preserved.

Portable observations, capacity counters, accounting, guards and hashes (`benchmark-results-record-reuse-2026-10-18.json`, removed). Three 376-file source archives, two patches, valid instrumented sources, the listing profiler, raw reports, runners and check/build logs are retained under ignored `benchmark-results/record-reuse-2026-10-18/`; binaries and large exports remain at inventoried temporary paths. Results cover one host with warm caches and uncontrolled desktop activity; initial observations are not cold-cache claims. Existing native timer contracts and default accounting policies do not establish equivalent work: Hahakigami truncates core milliseconds after root initialization, while OpenDisk includes engine initialization through scan/rollup return. App timing ends at CPU Canvas draw, and app RSS excludes external WebKit helpers.

## Tiny alias filtering and reader-count diagnostic — 18 October 2026

**No new production change retained.** An inode-range filter and conditional claim lookup reduced diagnostic finalization time, but the prototype tied the retained scanner in the native fixture cohort. It was rejected. The retained scanner, accounting checks, RAM optimization and previously validated signed app remain unchanged.

| 100,001-entry fixture, native scanner | Retained control | Alias prototype | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 132.0 ms | 132.0 ms | **131.460 ms** |
| Median process high-water RSS | 19.555 MiB | 19.438 MiB | 23.531 MiB |

The prototype wins **9/30 paired rounds** and remains **0.540 ms above OpenDisk** under the existing timer contracts. It fails the unchanged speed gate of a median at least 1 ms below OpenDisk and at least 18 wins, and shows no matched-control median improvement. The native RAM ratio is **0.9940**, within the 1.03 guard; this small RSS variation is not evidence of an algorithmic memory reduction. The retained control remains **16.90% below OpenDisk's native RSS**. Earlier near-tie results came from another cohort and are not precise evidence of cross-cohort progress or regression.

**Prototype and selectivity.** Tiny alias maps precompute inclusive minimum/maximum inode values. Queries outside that interval cannot match; surviving queries retain the full device/inode comparison. Claim-cursor lookup moves inside the alias-hit branch in both validation and pure planning. All metadata, raw-size, link-flag, error-order and later filesystem revalidation checks remain. The helper grows **72 → 88 bytes on the stack**; retained 80-byte records, 64-byte entry results, serialized layouts and heap structures stay unchanged. An independently checked existing native export places **99,979/99,987 regular files (99.992%)** outside the fixture's clone-alias interval, with three full identity hits. Current identities for those three owned fixture paths match the export; creation order alone was not used as evidence.

**Phase profiles.** Thirty-six observations cover control, conditional cursor only, range only and the combination: one initial and eight rotating warm rounds per variant. The combined candidate reduces validation's alias interval **211.896 → 127.959 µs**, planner alias checks **191.188 → 92.375 µs**, and diagnostic finalization **742.896 → 557.751 µs (−24.92%)**. Cursor-only validation instead increases to **216.896 µs**; all observations remain in the report. These elapsed `Instant` intervals are not CPU time, and instrumented totals are excluded from native acceptance. The diagnostic gain did not produce a whole-scan median gain.

**Reader-count experiment.** Thirty separate syscall-only observations compare one, two and three readers using the approved rich request, 256 KiB buffers, 16-byte alignment, fresh descriptors and the same three wide directories. One initial and nine rotating warm rounds per reader count produce wall medians **181.155 / 153.267 / 129.153 ms**. Summed thread CPU medians are **181.054 / 245.562 / 382.966 ms**; concurrent CPU sums are not wall latency. Every case covers 99,988 entries, 11,998,528 packed bytes and 51 calls, with all 60 power/thermal boundaries nominal. Two readers are slower than three in this diagnostic. No concurrency policy changed. This probe checks packed boundaries/masks without payload decoding or accounting and does not establish full-scanner performance.

**Accounting and custody.** All **93 new native observations** succeed, with a separate initial and 30 balanced warm rounds per engine, fresh processes/state, no retries, exclusions or cache purges. All **760 frozen inputs** stay unchanged; all **186 native power/thermal boundaries** are nominal with Low Power Mode off. All **62 Hahakigami scans** complete accounting without warnings, errors or exclusions: 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte clone deduction. OpenDisk reports 12,288 allocated bytes under its own default policy.

The canonical/range variants pass **196 scanner tests, 2 ignored**; cursor-only passes 193, with formatting and strict scanner/all-target Clippy clean before timing. Independent map/cursor/native-validation/planner oracles cover interval boundaries, arbitrary devices, borrowed reference identity, sparse queries, contradictory aliases, malformed claims, cancellation and metadata drift. High source, empirical and persistence reviews approve the correctness evidence and rejection. A duplicated instrumentation anchor stopped preparation before any incomplete build or timing; the corrected helper targets the named production validator, preserving the already compiled control's source bytes.

All 375 workspace inputs matched the frozen control before documentation, including the exact 20 previously validated scanner inputs and all 105 frontend inputs. The retained signed-app hash also matches. Previous **395 Rust tests / 22 ignored**, strict checks, signed-app/smoke validation, 147 observations, 98 Hahakigami accounting runs and eight exact export comparisons are reused only for that unchanged production scanner. They are not new measurements. Prepared app/larger-scope scripts were not executed after rejecting the prototype.

Portable observations, diagnostics, rejection, provenance and hashes (`benchmark-results-alias-filter-2026-10-18.json`, removed). Four 375-file source archives, three experimental patches, valid instrumented sources, raw reports, runners, probe source and check/build logs are retained under ignored `benchmark-results/alias-filter-2026-10-18/`; binaries and the referenced large export remain at inventoried temporary paths. Results cover one host with warm caches and uncontrolled desktop activity. Initial observations are not cold-cache claims. Hahakigami truncates core milliseconds after root initialization, while OpenDisk includes engine initialization through scan/rollup return; the unchanged contracts and default accounting policies do not establish equivalent work. Native RSS is a scanner-process measure. The normal app session is preserved.

## Hardlink owner election and sibling rollup — 18 October 2026

**Retained two finalization improvements; no credible OpenDisk speed win.** Hardlink planning now records each losing alias during the existing owner comparisons, then sorts the sparse IDs instead of scanning every node a second time. Rollup combines adjacent siblings with a common, strictly earlier parent before updating that parent once. Saturating arithmetic, path comparisons, owner ties, error order and partial updates remain equivalent to the original algorithms, including self/forward/invalid parents. No metadata checks, fields, record layouts, heap structures, dependencies, concurrency policy or timer boundaries changed.

| 100,001-entry fixture, native scanner | Retained control | Combined candidate | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 132.5 ms | 132.0 ms | 132.021 ms |
| Median process high-water RSS | 19.469 MiB | 19.469 MiB | 23.516 MiB |

The observed gain against the matched control is **0.5 ms**. The candidate's apparent **20.7 µs** median advantage over OpenDisk is below Hahakigami's truncated integer-millisecond precision and establishes no credible win. It wins **16/30 paired rounds**, failing the unchanged requirement for a median at least 1 ms below OpenDisk and at least 18 wins. Native RSS matches control exactly at the median and remains **17.21% below OpenDisk**. The earlier 0.346 ms gap came from a separate cohort; these results do not establish precise progress across cohorts.

**Profiling.** Thirty-six diagnostic observations cover control, one-pass hardlinks, sibling rollup and their combination: one initial and eight rotating warm rounds per variant. The combined candidate reduces the hardlink-planning interval **121.250 → 61.834 µs** and rollup **103.167 → 69.021 µs**; standalone variants independently improve their respective target phases. Combined finalization changes **865.146 → 754.146 µs (−12.83%)**. Unrelated validation, apply, publication and syscall variation receives no separate optimization credit. These are instrumented elapsed intervals, not CPU time; instrumented totals are excluded from native acceptance. No claim attributes the entire observed 0.5 ms native difference to these phase reductions.

**Regression guards.** On the **887,717-entry GitHub scope**, native medians change **3.955 → 3.911 s**, with native RSS **143.234 → 142.578 MiB**. Fixture app medians improve **162.110 → 161.624 ms**, while larger app medians increase **4.064042 → 4.065034 s (+0.024%)**. All four guards pass: at most 1.05× control for app/larger timing and 1.03× for larger native RSS. App-process RSS is mixed: fixture **110.453 → 111.344 MiB**, GitHub **240.563 → 238.219 MiB**. There is no universal or total-app RAM reduction claim.

**Accounting and verification.** All **147 timed observations** succeed: 93 fixture/native cases with 30 balanced warm rounds, followed by 54 app/larger cases with five rotating warm rounds per cell, each with a separate initial observation. All **758 native-stage and 1,133 remaining-stage inputs** stay unchanged; all **294 power/thermal boundaries** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings, errors or exclusions and agree across versions/layers. Fixture totals remain 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte clone deduction. OpenDisk reports 12,288 allocated bytes under its default policy. Eight complete native exports and eight exact path/identity/kind/raw-size comparisons pass before/after timing, with all **1,129 export inputs** stable across phases. No timed observations were retried or discarded.

The final source passes **395 Rust tests, 22 ignored**, with the benchmark feature. Formatting and strict workspace/all-target Clippy pass with default and benchmark features. Independent original-planner and reverse-rollup oracles cover saturation, chunk boundaries, all 2,401 four-node parent graphs, malformed metadata/path errors, dense/sparse compatibility and repeated aggregation. The isolated development-signed app passes frontend checking with zero errors/warnings, production build, signature verification and smoke. High source and evidence reviews approve correctness, source equivalence and the measured conclusions.

**Source custody.** A test-oracle type declaration initially triggered Clippy. The timed source remains frozen; the final validation source differs only by a local type alias inside `cfg(test)`, with no executable statement or production source change. The native scanner and signed app use the timed source; final tests and the exporter use the equivalent validation source. Both snapshots and the exact correction are preserved. Instrumentation setup stops, an unused incomplete build, the initial lint and a corrected report build-receipt assumption are recorded; they changed no timed observation. All 374 workspace inputs matched the final validation snapshot before documentation, and all 105 frontend inputs remain unchanged.

Portable observations, profiles, accounting, gates and hashes (`benchmark-results-aggregation-runs-2026-10-18.json`, removed). Five 374-file source archives, experimental/final patches, valid instrumented sources, raw reports, runners and check/build logs are retained under ignored `benchmark-results/aggregation-runs-2026-10-18/`; binaries and large exports remain at inventoried temporary paths. Results cover one host with warm caches and uncontrolled desktop activity. Existing timer contracts and default accounting policies do not establish equivalent work: Hahakigami truncates core milliseconds after root initialization, while OpenDisk includes engine initialization through scan/rollup return. Initial observations are not cold-cache claims. App timing ends at CPU Canvas draw, and app RSS excludes external WebKit helpers. The normal app session is preserved.

## Batch entry materialization — 18 October 2026

**Retained a batch construction improvement; the OpenDisk speed target remains unmet.** Each fully checked bulk batch now extends the listing through an exact-size mapped iterator, removing the capacity check from each entry append. Small reservations preserve the original geometric capacity progression; larger listings retain the 1.5× policy. Full-batch validation, accepted-prefix limits, error order, descriptor calls, clone ordinals, fallback and cancellation keep their previous behavior. No metadata fields, checks, layouts, dependencies or concurrency policy changed.

| 100,001-entry fixture, native scanner | Retained control | Batch materialization | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 135.0 ms | 134.0 ms | **133.654 ms** |
| Median process high-water RSS | 19.656 MiB | **19.445 MiB** | 23.609 MiB |

The candidate improves its matched control by **1 ms (0.74%)**, but remains **0.346 ms above OpenDisk** under the existing reported timer contracts. It wins **15/30 paired rounds**, failing the unchanged speed gate of a median at least 1 ms below OpenDisk and at least 18 wins. Native RSS is **1.07% below control** and **17.64% below OpenDisk**, passing the 1.03 RAM guard. Small RSS differences do not establish another algorithmic memory reduction. These are within-cohort observations; the earlier 0.607 ms gap came from a separate cohort.

**Diagnostics.** Fourteen separately instrumented observations, one initial and six alternating warm rounds per version, put median summed checked parsing/materialization at **2.840 → 2.811 ms (1.03% lower)**. Publication changes **1.914 → 1.922 ms** and finalization **0.851 → 0.838 ms** without changes to those algorithms; neither receives optimization credit. Assembly inspection of the paired diagnostic binaries confirms one batch capacity check outside the materialization loop and no per-entry output-capacity check. Seven directory-timeline observations put the warm median summed single-child dispatch gaps at **6.062 µs**, with **3.5 worker changes**. These instrumented elapsed intervals are neither CPU time nor exclusive scheduler cost; concurrent sums cannot be added to scanner latency. No scheduling variant was implemented, and instrumented totals are excluded from acceptance.

**Regression guards.** On the **886,945-entry GitHub scope**, native medians improve **3.955 → 3.871 s**, with native RSS **147.578 → 144.656 MiB (−1.98%)**. App medians change **3.958 → 3.946 s**. The fixture app median increases **163.100 → 164.524 ms (+0.87%)**, and GitHub app-process RSS increases **234.516 → 237.000 MiB (+1.06%)**. All four predeclared guards pass: at most 1.05× control for app/larger-scope timing and 1.03× for larger native RSS. The unfavorable app observations remain in the report; there is no universal app speed or RAM reduction claim.

**Accounting and verification.** All **147 timed observations** succeed: 93 fixture/native cases with 30 balanced warm rounds, followed by 54 app/larger-scope cases with five rotating warm rounds per cell, each with a separate initial observation. All **756 native-stage and 756 remaining-stage inputs** stay unchanged, and all **294 power/thermal boundaries** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings, errors or exclusions and agree across versions/layers. Fixture totals remain 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte clone deduction; OpenDisk reports 12,288 allocated bytes under its own default policy. Eight complete native exports and eight exact path/identity/kind/raw-size comparisons pass before/after timing, with all 752 export inputs stable across phases. No timed observations were retried or discarded.

The candidate passes **391 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy with the benchmark feature. Default-feature checks also pass. The isolated development-signed app passes frontend checking with zero errors/warnings, production compilation, signature verification and smoke. High source and evidence reviews approve the change and measured conclusions. Review corrected a report-writer timestamp index before publication; the valid raw traces were unchanged. An initial package-runner dependency safety stop occurred before the app build; the successful build used a documented per-invocation setting and the normal shared dependencies were untouched. Both logs are preserved.

Portable observations, profiles, accounting, failed speed gate and hashes (`benchmark-results-batch-materialization-2026-10-18.json`, removed). Two 373-file source archives, the retained patch, instrumented sources, raw streams, runners and check/build logs are retained under ignored `benchmark-results/batch-materialization-2026-10-18/`; binaries and large exports remain at inventoried temporary paths. All 373 workspace inputs matched the frozen candidate before documentation, including the reviewed 20 scanner inputs; all 105 frontend inputs remain unchanged. The normal app session is preserved. Results cover one host with warm caches and uncontrolled desktop activity. Hahakigami's integer core timer truncates milliseconds after root initialization, while OpenDisk includes engine initialization through scan/rollup return; the contracts and default accounting policies do not establish equivalent work. App timing ends at CPU Canvas draw, and app RSS excludes external WebKit helpers.

## Checked-name reuse and fixed-mask decoding — 18 October 2026

**No new production change retained.** Checked-name reuse and a specialized decoder reduced diagnostic parsing/materialization time, but the combined prototype tied the retained scanner in the native fixture cohort. The extra implementation was rejected; all retained scanner source, its native binary and its previously validated signed app remain unchanged.

| 100,001-entry fixture, native scanner | Retained control | Combined prototype | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 134.0 ms | 134.0 ms | **133.393 ms** |
| Median process high-water RSS | 19.578 MiB | 19.562 MiB | 23.742 MiB |

The prototype wins **14/30 paired rounds**, failing the unchanged speed gate of a median at least 1 ms below OpenDisk and at least 18 wins. Both Hahakigami medians remain about **0.607 ms above OpenDisk** under the existing reported timer contracts. The RAM guard passes, but the small RSS variation is not an algorithmic memory reduction. The retained control remains **17.54% below OpenDisk's native RSS**. Earlier gaps are from separate cohorts and are not precise evidence of improvement.

**Experiments.** A private transparent `NativeName` proof checked packed-name validity once and let entry construction reuse that proof. General constructors stayed checked. A separate header variant consolidated fixed-field bounds into a checked 48-byte prefix. A decoder variant additionally used a checked 104-byte prefix for exact full-mask rich regular-file payloads, retaining the original generic decoder for every other case. The combined variant preserved all native-name, signed-size, resource-fork, identity and accounting decisions, without unsafe code, dependencies or layout growth. Its independent test oracle is the original raw-byte decoder; comparisons include every decoded field, errors, `used`, allocation peaks, mask combinations, truncations, corruptions, dot names and native bytes. The combined prototype passes **190 scanner tests, 2 ignored**; each standalone prototype passes 188, with formatting checks clean. High source review approved correctness; performance rejection is separate.

**Diagnostic results.** The first 36-observation comparison has one initial and eight rotating warm rounds per variant: summed checked parsing/materialization is **2.894 ms** for control, **2.781 ms** for name-proof reuse, **2.962 ms** for header-only and **2.731 ms** for full-mask decoding. A second 36-observation comparison puts control at **2.972 ms** and the combined variant at **2.771 ms (6.77% lower)**. Publication and finalization variation receives no separate optimization credit. These are elapsed `Instant` intervals, not CPU time; concurrent directory sums are not scanner critical-path time. Neither diagnostic cohort substitutes for the native acceptance result.

Seven separately instrumented allocation observations measure median summed growth intervals of **0.182 ms** for implicit listing growth, **0.446 ms** for explicit listing reservation, **0.073 ms** for the name arena and **0.024 ms** for retained record chunks. Their additional clocks and branches perturb total timings, so those totals are excluded from other comparisons. Existing payload bytes at growth are not actual copied bytes: the allocator can grow in place. The measurements also exclude page touches while writing records. Existing native telemetry puts the median interval after the latest recorded listing until enumeration join at only **33.5 µs** for control and **27 µs** for the prototype. Summed worker idle waits are concurrent and do not add to scan latency. All **79 diagnostic observations** are preserved.

**Accounting and custody.** All **93 new native observations** succeed, with one separate initial and 30 balanced warm rounds per engine, fresh processes/state, no retries, exclusions, cache purges or builds during timing. All **754 frozen inputs** stay unchanged; all **186 power/thermal boundaries** are nominal with Low Power Mode off. All **62 Hahakigami scans** complete accounting without warnings, errors or exclusions: 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte clone deduction. OpenDisk reports 12,288 allocated bytes under its own default policy.

All 372 workspace inputs matched the frozen control before documentation, including the exact 20 retained scanner inputs. The retained native and signed-app binaries match the previous validated hashes. The previous **389 Rust tests / 22 ignored**, strict checks, signed-app validation, 147 observations, 98 Hahakigami accounting runs and eight exact native export comparisons are therefore reused for the unchanged production scanner. They are not presented as new measurements. No new app/larger-scope cohort or exports were run after rejecting the prototype. High evidence review independently verified the new native/diagnostic data, rejection, tail calculations, hashes and validation reuse.

Portable observations, variants, profiles, failed speed gate and hashes (`benchmark-results-decoding-2026-10-18.json`, removed). Five 372-file source archives, four experimental patches, instrumented sources, raw reports, runners and test/build logs are retained under ignored `benchmark-results/decoding-2026-10-18/`; binaries remain at inventoried temporary paths. Each variant was rebuilt in a separate target after clearing its scanner package. Results cover one host with warm caches and uncontrolled desktop activity. Existing timer boundaries and default accounting policies do not establish equivalent work: Hahakigami reports truncated integer core milliseconds after root initialization, while OpenDisk includes engine initialization through scan/rollup return. The normal app session and preexisting frontend edits are preserved.

## Single-pass native-name validation — 18 October 2026

**Retained a small validation improvement; the OpenDisk speed target remains unmet.** Bulk-record parsing and the checked short-name constructor now test for NUL and slash bytes in one iteration instead of two separate searches. Length, terminator, dot-name, bounds and fallback checks remain identical. This changes only two predicates in `bulk.rs` and `entry_path.rs`, adds no storage or dependency, and preserves all accounting checks.

| 100,001-entry fixture, native scanner | Previous retained scanner | Single-pass validation | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 134.0 ms | 133.5 ms | **132.874 ms** |
| Median process high-water RSS | **19.539 MiB** | 19.594 MiB | 23.656 MiB |

The observed gain against the matched control is **0.5 ms**, leaving a **0.626 ms reported gap** to OpenDisk. The candidate wins **13/30 paired rounds**, failing the unchanged requirement for a median at least 1 ms below OpenDisk and at least 18 wins. The control's gap is 1.126 ms in this cohort; the previous 0.859 ms gap came from another cohort. Native RSS rises **0.28%** against control, within the 1.03 guard, and remains **17.17% below OpenDisk**. This preserves the RAM advantage without demonstrating a further memory reduction.

**Profiling and rejected alternatives.** An initial seven-run diagnostic puts clone preparation at about **0.088 ms**, limiting its potential benefit. A four-variant comparison, with one initial and eight rotating warm rounds per variant, reduces summed checked parsing/materialization from **3.400 to 2.941 ms** with single-pass name validation. Caching the earliest sparse-probe queue index gives no publication benefit: **1.906 → 1.928 ms** alone, or **1.898 → 1.943 ms** when added to the name change. The cache was removed. A separate 21-observation comparison measures **2.836 ms** for the simple loop versus **2.953 ms** for `memchr2` from the already locked 2.8.3 library. It fails the predeclared requirement to be at least 10% faster than the loop; no direct dependency is retained. All **64 diagnostic observations** and all five source variants are preserved. These are elapsed `Instant` intervals, not CPU time or acceptance timings; concurrent directory sums cannot be added to scanner time.

**Regression observations.** On the **885,700-entry GitHub scope**, native medians are **3.906 → 3.841 s**, with RSS **140.969 → 143.625 MiB (+1.88%)**. App medians are **3.977 → 4.001 s (+0.61%)**, while fixture app medians are **162.80 → 163.24 ms (+0.27%)**. All four predeclared guards pass: at most 1.05× control for app/larger-scope time and 1.03× for larger native RSS. Slower app medians and increased RSS remain in the evidence; the fixture gain is not a general speed guarantee.

**Accounting and verification.** All **147 chosen observations** are valid: 93 fixture/native cases with 30 balanced warm rounds, followed by 54 app/larger-scope cases with five rotating warm rounds per cell, each with a separate initial observation. All **752 native-stage and 752 remaining-stage inputs** stay unchanged, and all **294 power/thermal boundaries** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings or exclusions, with equal counts, attributed allocated totals and deductions across versions and layers. Eight complete native exports and eight exact path/identity/kind/raw-size comparisons pass before/after timing; all 749 export inputs remain stable. OpenDisk's app discovery counter still excludes the root, with raw values preserved.

The selected source passes **389 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy. The isolated development-signed app passes frontend checking, compilation, signature verification and smoke. High source and evidence reviews approve the two predicate rewrites and the measured conclusions. The normal app session and preexisting frontend edits are preserved.

Portable observations, variants, profiles, gates, accounting and hashes (`benchmark-results-name-validation-2026-10-18.json`, removed). Five 371-file source archives, variant patches, raw reports, runners and check/build logs are retained under ignored `benchmark-results/name-validation-2026-10-18/`; large exports and binaries remain at inventoried temporary paths. Each compiled variant uses a separate target and forced scanner-package rebuild. Review caught and corrected an archive-preflight edge before publication; calculations and final JSON are now validated before staged output is published. No benchmark observations failed, were retried or were discarded.

Timer contracts and default accounting policies remain unchanged and do not establish equivalent work. Hahakigami reports truncated integer milliseconds after root initialization through final aggregation/state update; OpenDisk includes engine initialization through scan/rollup return. Results cover one host with warm caches and uncontrolled desktop activity. App timing ends at CPU Canvas draw, and app RSS omits external WebKit helpers.

## Borrowed publication and validated basenames — 18 October 2026

**Retained a small publication improvement; the OpenDisk speed target remains unmet.** Compact publication now borrows its temporary `ScannedNode` rather than passing its 104 bytes by value. Short names use the existing checked single-component invariant to return native bytes directly. Owned paths keep the same `Path::file_name().unwrap_or(as_os_str())` behavior. The tree still copies retained names, and temporary paths are explicitly dropped before error handling and clone publication, preserving the previous release point.

| 100,001-entry fixture, native scanner | Previous retained scanner | Borrowed publication | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 132.0 ms | 131.5 ms | **130.641 ms** |
| Median process high-water RSS | 19.586 MiB | **19.500 MiB** | 23.492 MiB |

The candidate gains **0.5 ms against its matched control**, leaving a **0.859 ms gap** to OpenDisk in this cohort. It wins **8/30 paired rounds** and fails the unchanged gate requiring a median at least 1 ms below OpenDisk and at least 18 wins. The control's gap is 1.359 ms here; the earlier 1.731 ms gap is from a different cohort and is not a precise improvement baseline. Native RSS is 0.44% below control, within the 1.03 guard, and remains **16.99% below OpenDisk**. Small observed RSS changes do not establish an additional memory reduction.

**Profile.** Fourteen separate instrumented observations, with one initial and six balanced alternating warm rounds per version, put median summed publication duration at **2.396 → 1.805 ms (24.6% lower)**. Finalization stays **0.835 → 0.864 ms**, and summed checked parsing/materialization is **3.300 → 3.422 ms**; neither phase was optimized. The longest wide-directory bulk-read interval is about **127.20 → 127.31 ms**, with longest checked materialization **1.15 → 1.19 ms**. These are `Instant` elapsed intervals, not CPU time. Concurrent directory intervals overlap and are not additive with publication or scanner time. Instrumented totals are excluded from acceptance. OpenDisk also serializes tree publication under a mutex; it does not publish whole subtrees concurrently.

**Checks and storage.** Checked name constructors, every compact-record metadata/topology check, descriptor validation, clone/resource-fork accounting, cancellation, parsing, metadata requests/flags, workers and aggregation remain intact. Only `entry_path.rs`, `tree.rs` and publication call sites in `lib.rs` change. `EntryPath` remains 24 bytes, `Result<Entry>` 64 bytes, `ScannedNode` 104 bytes, and retained records 80 bytes. Tests compare native-name semantics at short/owned boundaries, Unicode and invalid UTF-8, special paths and NUL bytes; they also verify invalid publication is rejected before mutation and retained names survive mutation/drop of their temporary source.

**Regression observations.** On the **884,963-entry GitHub scope**, native medians are **3.882 → 3.899 s (+0.44%)**, with RSS **146.516 → 146.844 MiB (+0.22%)**. App medians are **4.011 → 4.008 s**; fixture app medians are **161.90 → 164.60 ms (+1.67%)**. All four predeclared guards pass: app/larger-scope time at most 1.05× control and larger native RSS at most 1.03×. The slower fixture app and larger native observations remain in the evidence; this is not a general app-speed claim.

**Accounting and verification.** All **147 chosen observations** are valid: 93 fixture/native observations with 30 balanced warm rounds, followed by 54 app/larger-scope observations with five rotating warm rounds per cell, each with a separate initial observation. All **750 native-stage and 750 remaining-stage inputs** stay unchanged; all **294 power/thermal boundaries** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings or exclusions, with equal counts, attributed allocated totals and clone deductions across versions/layers. Eight complete exports and eight exact native path/identity/kind/raw-size comparisons pass before/after timing on both scopes. OpenDisk's app counter continues to exclude the root; raw values are preserved.

The candidate passes **389 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy. The isolated development-signed app passes frontend checking, production build, signature verification and smoke. High source and evidence reviews approve the candidate and calculations. The normal app session and all preexisting frontend edits are preserved.

Portable observations, profiles, gates, accounting and hashes (`benchmark-results-publication-abi-2026-10-18.json`, removed). Two 370-file source archives, the patch, raw reports, runners and build/check logs are preserved under ignored `benchmark-results/publication-abi-2026-10-18/`; large exports and binaries remain at inventoried temporary paths. Each diagnostic uses a separate target and forced scanner-package rebuild. A source-preparation assertion caught a damaged instrumentation anchor before measurement; its partial source is retained. No benchmark observations failed, were retried or were discarded.

Timer contracts remain unchanged: Hahakigami's integer-millisecond core timer starts after root initialization and ends after final aggregation/state update; OpenDisk's timer starts before engine initialization and ends after scan/rollup returns. No setup costs are subtracted, and default accounting policies do not establish equivalent work. Results cover one host with warm caches and uncontrolled desktop activity. App timing ends at final CPU Canvas draw; GPU presentation and total app/WebKit memory remain unmeasured.

## Clone-accounting lookup cost — 18 October 2026

**Retained a focused accounting optimization; OpenDisk remains faster on the native fixture.** Every file still passes both clone-alias validation sweeps. For alias maps containing at most four identities, a private `AliasLookup` now compares borrowed full `(device, inode)` keys stored on the stack. Larger maps retain the original `HashMap::get`. The original maps, insertion semantics, capacities, release points and heap-scratch accounting stay intact. No per-node or additional heap storage is introduced.

| 100,001-entry fixture, native scanner | Previous retained scanner | Tiny-alias lookup | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 137.5 ms | 136.0 ms | **134.269 ms** |
| Median process high-water RSS | 19.539 MiB | 19.758 MiB | 23.469 MiB |

The candidate gains **1.5 ms (1.09%) against its matched control**, leaving a **1.731 ms gap** to OpenDisk. It wins **11/30 paired rounds**, failing the unchanged target of a median at least 1 ms below OpenDisk and at least 18 wins. The matched control's gap is 3.231 ms in this cohort; earlier absolute gaps are not precise cross-cohort improvement evidence. Native fixture RSS rises **1.12%** against control, within the 1.03 guard, and remains **15.81% below OpenDisk**. This preserves the RAM advantage; it does not demonstrate a further RAM reduction.

**Profile and selected scope.** One separate initial plus six balanced alternating warm rounds per version puts median finalization elapsed time at **2.052 → 0.910 ms (55.7% lower)**. Native alias validation falls **0.772 → 0.241 ms**, and pure allocation-plan alias validation falls **0.758 → 0.201 ms**. Sparse member validation remains **95 → 96 µs**, with the same three claims and three direct revalidation queries. These are instrumented `Instant` elapsed durations, not thread CPU time or acceptance timings. An earlier seven-run baseline profile identified the dense membership sweeps as the target; reset and rollup together cost only about 0.24 ms there. Aggregation code remains unchanged, and its timing variation receives no separate algorithmic credit.

**Checks retained.** Native validation must detect mutation; the public allocation planner must independently validate saved metadata before snapshot mutation. Both sweeps remain, with claim-cursor advancement in the same order and all missing-claim, hardlink-flag, size and full identity comparisons unchanged. Resource-fork/data-fork bounds, group rejection, descriptor/parent checks, cancellation and stable ownership remain intact. The change touches only `clones.rs` and `clones/accounting.rs`. Parsing, publication, aggregation, metadata requests/flags, worker policy, 80-byte records, and public/archive/cache formats are unchanged. Differential tests cover empty maps, both sides of the four-key threshold, excess capacity, misses, equal inode numbers on different devices, extreme keys, duplicate insertion, non-Copy values and reference identity.

**Regression observations.** On the 884,237-entry GitHub scope, native medians are **4.003 → 3.983 s**, while native RSS is **141.02 → 143.31 MiB (+1.63%)**. App medians are **4.081 → 4.110 s (+0.71%)**; fixture app medians are **165.08 → 168.74 ms (+2.22%)**. All predeclared timing guards (at most 1.05× control) and native RSS guards (at most 1.03×) pass. The higher app medians and native RSS are retained in the evidence. Uncontrolled desktop activity and overlapping ranges prevent a general app-speed claim. App RSS omits external WebKit helpers; no total-app memory claim follows.

**Accounting and verification.** The chosen comparison contains **147 successful observations** in two stages: 93 fixture/native observations with 30 balanced warm rounds, then 54 app/larger-scope observations with five rotating warm rounds per cell. All cells also have a separate initial observation. All **748 native-stage and 749 remaining-stage inputs** stay unchanged; all **294 power/thermal observations** are nominal with Low Power Mode off. All **98 Hahakigami scans** finish accounting without warnings or exclusions. Counts, attributed allocated bytes and deductions agree across versions and layers. Eight complete exports and eight exact native path/identity/kind/raw-size comparisons pass before/after timing, covering 100,001 and 884,237 entries. OpenDisk's app discovery counter continues to exclude the root, with its raw counts preserved.

The candidate passes **388 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy. The isolated development-signed app passes frontend checking, compilation, signature verification and smoke. High source review approved the lookup and both callers; final High review independently verified every chosen observation, accounting parity, exports, source/artifact hashes and report calculations. No normal app was replaced or relaunched, and no privacy/keychain settings changed.

Portable observations, phase profiles, gates, accounting and hashes (`benchmark-results-finalization-2026-10-18.json`, removed). Two manifest-listed source archives, the patch, raw reports, runners and check/build logs are retained under ignored `benchmark-results/finalization-2026-10-18/`. Binaries and large raw exports remain at inventoried `/private/tmp` locations. Each diagnostic variant used a separate target and a forced scanner-package rebuild. One source-preparation assertion caught a nonunique instrumentation anchor before compilation/measurement; its partial source is retained. There were no failed, retried or discarded benchmark observations. Results describe one host with warm caches and each app's default accounting policy; GPU presentation and strict equivalent work against OpenDisk remain unestablished.

## Smaller tree-publication overhead — 18 October 2026

**Retained a publication improvement; the OpenDisk speed target remains unmet.** The original checked parser stays intact. Publication now computes invariant root/capacity policy once per listing, generates its dense child-ID range directly into retained adjacency, and combines the already accumulated saturating byte totals once per batch. The existing store write lock makes those child IDs contiguous, including filtered, empty, cancelled and capacity-limited prefixes. Generic child publication remains available for legacy/incremental callers. A wide directory no longer needs a second roughly 130 KiB child-ID scratch vector.

| 100,001-entry fixture, native scanner | Previous retained scanner | Publication candidate | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 138 ms | 137 ms | **134.471 ms** |
| Median process high-water RSS | 19.648 MiB | **19.578 MiB** | 23.531 MiB |

The candidate's median is **0.72% lower than our control**, but remains **1.88% above OpenDisk** and wins only **11/30 paired rounds**. The predeclared speed gate requires a median at least 1 ms below OpenDisk and at least 18 paired wins; both conditions fail. Native fixture RSS remains **16.80% below OpenDisk**, with a candidate/control ratio of 0.9964, inside the 1.03 RAM guard. These small whole-scan differences are observations on one host, not a general speed guarantee.

**Where time changed.** A separate instrumented comparison has one initial plus six alternating warm rounds per version. Median summed per-directory publication duration falls **3.827 → 2.456 ms (35.8%)**. These are elapsed `Instant` intervals summed across commits, **not thread CPU time**. Checked parsing plus materialization stays essentially unchanged at **1.152 → 1.158 ms per wide directory**. Instrumented totals are excluded from performance acceptance. The retained change affects only `lib.rs` and `tree.rs`; metadata requests, flags, buffers, descriptor checks, clone/resource-fork validation, worker policy, 80-byte records, and public/archive/cache formats are unchanged.

**Rejected experiments.** Fusing checked parsing with private entry construction removed temporary decoded-record vectors, but its separate 93-run cohort tied its control at 137 ms, and fresh phase measurements showed no processing gain. It was dropped to keep the change focused on publication. Disassembly also identified an out-of-line helper copying a 104-byte value; forced inlining did not improve fresh diagnostic publication duration, so that hint was dropped. These cohorts are recorded separately; their absolute times are not substituted for the final comparison.

**Larger scope and app guards passed.** On the 883,207-entry GitHub scope, native medians are **3.940 → 3.866 s**, with native RSS **148.42 → 146.20 MiB**. App scan-through-final-CPU-draw medians are **3.996 → 3.971 s**. Fixture app medians are **170.96 → 167.35 ms**. All predeclared timing guards (candidate at most 1.05× control) and native RAM guards (at most 1.03×) pass. App-process RSS also stays close to control, but external WebKit helpers and GPU presentation remain unmeasured; no total-app memory claim follows.

**Accounting and verification.** The chosen comparison contains **147 successful observations** in two stages: 93 fixture/native observations, then 54 app/larger-scope observations after building the isolated app. Every cell has a separate initial observation; fixture/native has 30 balanced warm rounds, and the remaining cells have five rotating warm rounds. All **746 native-stage and 749 remaining-stage inputs** stay unchanged, and all **294 power/thermal observations** are nominal with Low Power Mode off. All **98 Hahakigami scans** complete accounting without warnings or exclusions. Both versions and both layers agree on counts, attributed allocated bytes and full-clone deductions. Eight complete native exports and eight exact path/identity/kind/raw-size comparisons pass before/after timing, covering 100,001 fixture and 883,207 GitHub entries. OpenDisk's app discovery counts exclude the root and remain unmodified.

The retained source passes **387 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy. The isolated development-signed app passes frontend checking (zero errors/warnings), production compilation, signature verification and smoke. Max source review approved the publication code, and High review independently verified all chosen observations, accounting, exports, artifact hashes and report calculations. The normal app was not replaced or relaunched, and no privacy/keychain settings changed.

**Diagnostic corrections are preserved.** An initial shared-target build reused an artifact across source roots; those profiles are marked invalid. Each valid diagnostic was rebuilt in a separate target after clearing its scanner package, with distinct stable binary hashes. An app-harness preflight also stopped before scanning because sampler artifacts made its private state nonempty. The corrected harness uses separate empty app-state and measurement-artifact directories. The failed preflight, invalid profiles and raw streams remain in the evidence; no successful timing was removed or retried.

Portable observations, gates, accounting, source hashes and diagnostic corrections (`benchmark-results-publication-2026-10-18.json`, removed). Raw reports, four frozen source archives, retained/rejected patches, runners and build/check logs are preserved under ignored `benchmark-results/publication-2026-10-18/`. Large raw exports and binaries remain at the inventoried `/private/tmp` paths. OS caches and desktop activity are uncontrolled. OpenDisk retains its default accounting policy, so this is not proof of equivalent work between the two apps.

## Inside `getattrlistbulk` — 18 October 2026

**No new speed win; the retained scanner is unchanged.** Kernel profiling now identifies the dominant shared cost: APFS's native bulk reader performs inode and B-tree lookups for both scanners. The smaller OpenDisk attribute request does not avoid that path. A disposable flag-only candidate failed the full-scanner speed gate and was rejected, preserving the existing accounting checks and RAM optimization.

| 100,001-entry fixture, native scanner | Retained scanner | Remove bulk `NOFOLLOW` | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 138 ms | 139 ms | **133.421 ms** |
| Warm range, all observations retained | 135–159 ms | 136–246 ms | 131.954–142.394 ms |
| Median process high-water RSS | **19.547 MiB** | 19.555 MiB | 23.633 MiB |

The candidate wins **4/30 paired rounds** against OpenDisk. The predeclared gate required at least 18 wins and a median at least 1 ms below OpenDisk; neither condition passes. Its RAM ratio to the retained scanner is 1.0004, within the 1.03 guard. The retained scanner remains **3.43% slower** and uses **17.29% less native peak RSS** than OpenDisk in this cohort. This is a fresh comparison; absolute timings from older cohorts are not interchangeable.

**Kernel evidence.** Three four-second Time Profiler recordings use the supported **Record Kernel Callstacks** option on an owned diagnostic process. Rich Hahakigami and OpenDisk-equivalent requests both show `apfs_vnop_getattrlistbulk` in over 99.8% of sampled running execution. Approximately 76–77% of samples include inode lookup and 66% include B-tree lookup with three readers. These inclusive categories overlap. About 31% end in functions whose names contain `lock` or `mutex`, versus 9.5% with one reader. This measures execution within lock/reference functions; it is **not blocked time or proof of a particular contended lock**. The two three-reader profiles are very similar. No privacy/security settings changed. Recording-limit termination is expected, and these traces are excluded from performance acceptance.

**Attribute controls.** A separate C probe ran **180 observations**: nine request variants, one/three readers, one initial and nine rotating warm rounds. Each covers only the three wide directories (**99,988 entries**), using fixed 256 KiB buffers and fresh independent directory descriptors. It validates record boundaries and returned masks, without the scanner's payload decoding, traversal or accounting. Full Hahakigami attributes emit 11,998,528 packed bytes in 51 calls, versus OpenDisk's 7,199,104 bytes in 33 calls, including EOF calls. Removing field groups is not monotonically faster: three-reader probe wall medians are 131.174 ms for the full request and 134.611 ms for OpenDisk's request. Summed concurrent syscall medians are 392.280 and 402.891 ms respectively; those sums are not wall times. These diagnostic controls do not establish a full-scanner win or a safe attribute omission.

**Rejected change.** Apple's [bulk-call contract](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/man/man2/getattrlistbulk.2) already returns metadata for a symlink itself. The isolated candidate therefore changed only rich bulk flags `0x21 → 0x20`; the basic fallback, descriptor no-follow checks, all metadata fields and accounting remained intact. It did not improve full-scanner performance. No flags affecting metadata freshness were tested or adopted. Source inspection also found no scheduler/store lock between consecutive bulk calls. Streaming publication would require changing the existing private-listing rollback contract; it was not implemented.

**Accounting and verification.** All **93 full-scanner observations** succeed: one separate initial plus 30 balanced cyclic warm rounds per engine, fresh process/state, no retries, exclusions, cache purges, builds or profiling during timing. All **744 frozen inputs** remain unchanged. All 186 full-scanner and 360 attribute-probe power/thermal observations are nominal with Low Power Mode off. Every one of the **62 Hahakigami scans** reports complete accounting with no warnings: 100,001 entries, 8,192 attributed allocated bytes and a 4,096-byte full-clone deduction. OpenDisk consistently reports 12,288 allocated bytes. A High reviewer independently verified probe fidelity, all diagnostic rows, kernel summaries and the full-scanner evidence, including seven failure-handling checks. The current scanner's source bytes match the frozen retained version. No new per-node exports, app build or large-scope cohort were needed after rejecting the candidate; earlier source/accounting validation remains applicable to the unchanged scanner.

Portable evidence, every observation, kernel summaries, hashes and failed gates (`benchmark-results-getattrlistbulk-2026-10-18.json`, removed). Frozen sources, the rejected patch, probe source/build receipt, raw results, compressed profile exports and the runner are retained under ignored `benchmark-results/bulk-2026-10-18/`. Large traces and binaries remain in the recorded `/private/tmp` locations. The probe can be rebuilt with `xcrun clang -O3 -Wall -Wextra -Werror -pthread benchmark-results/bulk-2026-10-18/probe/bulk_probe.c -o /private/tmp/hahakigami-bulk-probe`; its receipt documents arguments and measurement boundaries. Native RSS remains a scanner-process measure, not total desktop-app/WebKit memory or physical footprint. One host with warm caches does not establish cold-cache or all-hardware performance. The normal app was not rebuilt, installed or relaunched.

## Scan-speed attempt with accounting intact — 17 October 2026

**The OpenDisk speed target remains unmet.** The retained scanner has a lower observed fixture median and lower native peak RSS than our previous RAM-optimized version, but OpenDisk remains faster in the scanner-only fixture test.

| 100,001-entry fixture, native scanner | Previous RAM-optimized baseline | Retained candidate | OpenDisk |
| --- | ---: | ---: | ---: |
| Warm median | 142 ms | 139 ms | **136.940 ms** |
| Warm range | 137–150 ms | 136–146 ms | 131.870–144.228 ms |
| Median process high-water RSS | 21.11 MiB | **19.48 MiB** | 23.52 MiB |

The candidate's median is **2.11% lower than our control**, but **1.50% higher than OpenDisk**. It wins only **8 of 30 paired rounds**. Before final timing, the protocol required a median at least 1 ms below OpenDisk, to clear native integer-millisecond truncation, and at least 18/30 paired wins. Both speed conditions fail. Overlapping ranges and uncontrolled desktop load preclude a general speed guarantee. Initial diagnostic cohorts are retained and are not substituted for this result.

**Retained changes.** Temporary native basenames up to 22 bytes now stay inline in a private `EntryPath`; longer and full paths retain owned `Box<Path>` storage. Both `Entry` and `ScannedNode` preserve that representation through publication, removing short-name allocations and frees. The path slot remains 24 bytes and `io::Result<Entry>` remains 64 bytes. Final aggregation writes only aggregate allocated sizes, keeping the complete allocation-plan validation, reverse-parent order, saturation and malformed-parent behavior. No unsafe code, dependencies, requested metadata, syscall flags, buffer policy, scheduling, retained records, or public/archive/cache formats changed.

**Accounting and RAM stayed intact.** Native fixture RSS falls a further **7.74%** from the preceding RAM-optimized control, and is **17.18% below OpenDisk** in this cohort. GitHub native RSS falls **147.45 → 142.20 MiB (3.56%)**. All timed Hahakigami counts, attributed allocated totals and clone deductions agree; clone checks complete without warnings. Eight exact native path/identity/kind/raw-own-size cross-version and temporal comparisons pass, with complete exports of 100,001 fixture and 882,694 GitHub entries.

**Regression observations.** GitHub native medians are 3.975 → 3.955 s; app medians are 4.000 → 4.024 s (+0.61%). Fixture app scan-through-final-CPU-draw medians are 167.25 → 166.94 ms. Native RAM and the predeclared large-scope timing guards pass. GitHub app-process high-water RSS rises slightly, 240.11 → 240.42 MiB (+0.13%); no universal app-memory reduction is claimed. External WebKit helpers and GPU presentation remain unmeasured.

**Investigation and rejected experiments.** A disposable instrumented build put over 98% of wide-listing enumeration time inside bulk read syscalls: approximately 134 ms reading, 0.4–0.46 ms parsing and 0.7–0.97 ms materializing a 33k-entry directory. Those timings are diagnostic only. Uninstrumented 64/128/512 KiB buffer-cap variants did not produce a useful consistent win, so the existing 64→256 KiB policy remains. Explicit user-initiated QoS and three/six-worker diagnostics did not help, so scheduling remains unchanged. A marginal early inline-name pilot was superseded by the balanced final cohort. A later checked-name/publication/cursor shortcut experiment recorded 139 ms versus OpenDisk's 133.321 ms, winning 1/15 pairs; it was reverted. Its source, pilot and patch remain available. Metadata freshness or clone-check reductions were not adopted.

**Verification.** The frozen cohort contains **147 successful observations**, with a separate initial run per cell, 30 balanced fixture/native warm rounds (each engine occupies each position ten times), and five warm rounds in the other cells. There are no timed retries, exclusions, cache purges or concurrent builds. All **393 source/binary/configuration inputs** remain unchanged; all 294 power/thermal observations are nominal with Low Power Mode off. The retained candidate passes **386 Rust tests, 22 ignored**, formatting and strict workspace/all-target Clippy. Its isolated development-signed app passes source checking, production compilation, signature verification and smoke. Source Max review found no correctness defects; High empirical review verified the evidence while explicitly leaving performance acceptance unmet. After rejecting the final micro-experiment, current sources were restored byte-for-byte to the measured/tested candidate; unrelated UI edits remain intact.

Portable results, every observation, paired wins, source hashes and failed speed gate (`benchmark-results-scan-speed-2026-10-17.json`, removed). Raw results, all pilots, frozen baseline/candidate sources, rejected source and logs are retained under ignored `benchmark-results/scan-speed-2026-10-17/`. Large raw exports and build artifacts remain in the recorded `/private/tmp` locations. The code is retained in the working tree for review; **no normal app update was built, installed or relaunched**, and no privacy/keychain settings changed.

## Lower temporary listing memory with native basenames — 17 October 2026

**Kept as a memory optimization; no speedup is demonstrated.** Bulk enumeration now retains each ordinary file's native basename instead of allocating its full path. Full paths are reconstructed only for startup firmlink matching, directory policy, incremental root comparison and errors. Directory/symlink descriptor checks, clone attributes and revalidation, worker policy, 80-byte retained records, and public/archive/cache formats are unchanged.

**Why OpenDisk had an advantage.** Its source retains less per-file information and requests fewer bulk attributes. Hahakigami retains full identities, allocated sizes, raw and aggregate values, and clone accounting. The avoidable cost was temporary full paths that were discarded when the compact tree copied only the basename. Across the fixture's 99,987 regular files, full path byte lengths total 10,131,970 bytes versus 1,099,861 basename bytes: **9,032,109 repeated prefix bytes**. This is a byte-length sum, not a simultaneous allocation or allocator-overhead measurement. Basenames still have their own allocations; this change does not introduce a packed-name arena.

The earlier fixture diagnostics put clone revalidation at 56–78 microseconds, versus 40–51 ms of enumeration wall time. Three workers performed the substantive listings while three were mostly idle. Separate rotating diagnostics found essentially equal three/six-worker timings; one/two workers were slower. The worker policy was retained. Remaining timing differences were not isolated to a particular metadata attribute or instruction.

**Fresh comparison.** The same M4 Pro host and upstream OpenDisk binary were used. Frozen baseline and candidate CLI/app builds each use six workers; OpenDisk selects three. There are **72 successful timed observations**, one separate initial plus five rotating warm rounds per engine/layer/scope, with no retries, excluded timings, cache purges or simultaneous builds. All **392 inputs** remained unchanged, and all 144 power/thermal observations were nominal with Low Power Mode off. Absolute fixture times shifted for both apps since the preceding comparison; only ratios within this cohort are interpreted.

Warm medians; RAM is native CLI process high-water RSS:

| Measurement | Before | Candidate | OpenDisk |
| --- | ---: | ---: | ---: |
| Fixture, scanner peak RSS | 29.42 MiB | **21.38 MiB** | 23.50 MiB |
| Fixture, native scan | 133 ms | 133 ms | 129.49 ms |
| Fixture, app scan + final CPU draw | 156.85 ms | 159.58 ms | 177.10 ms |
| GitHub, scanner peak RSS | 153.19 MiB | **146.73 MiB** | 261.98 MiB |
| GitHub, native scan | 3.944 s | 3.924 s | 4.428 s |
| GitHub, app scan + final CPU draw | 3.941 s | 3.967 s | 4.520 s |

Native fixture RSS falls **27.35%**, now **9.04% below OpenDisk**; GitHub native RSS falls **4.21%**. Native fixture timing is unchanged. GitHub native timing changes by −0.51%, while app medians change by +1.74% on the fixture and +0.64% on GitHub. Baseline/candidate ranges overlap in each timing cell, so no meaningful speed improvement is claimed. App native-process RSS also falls, but external WebKit helpers remain unaccounted for; no total-app memory claim follows.

**Correctness and checks.** Eight untimed exports cover both versions before and after timing. All eight cross-version/temporal comparisons match every native path, kind, identity and raw own size, with complete footers and no unknown fields. The fixture has 100,001 root-inclusive entries; GitHub has 882,642. Every timed baseline/candidate attributed allocated total agrees, as do clone deductions; all accounting checks complete without warnings. App per-node attribution is not independently exported. Release baseline/debug candidate exporters are used only for untimed correctness checks.

The candidate passes **382 Rust tests, 22 ignored**, workspace/all-target performance-feature checks, strict Clippy and formatting. The first restricted run's 17 native app failures were sandbox-denied home temporary directories/event-stream setup; the unchanged candidate passed on the host rerun, and both logs are retained. The isolated development-signed app passes framework checking (zero errors/warnings), production compilation, signature verification and its smoke check. Independent Max review approved path/traversal/incremental safety without correction rounds; ordinary High review verified the empirical calculations, with the last temporal result subsequently verified by Main.

Portable samples, ratios, checks and hashes (`benchmark-results-fixture-optimization-2026-10-17.json`, removed). Raw reports, pilots, logs, the patch, runner and frozen candidate source are retained under ignored `benchmark-results/fixture-optimization-2026-10-17/`; large raw exports and binaries remain in the recorded `/private/tmp` locations. The code change is in the working tree. No normal app update was built, installed or relaunched, and no privacy/keychain settings changed.

## Current working tree versus latest Radix and OpenDisk — 17 October 2026

Hahakigami has the lowest observed warm app scan-through-draw time on both workloads. OpenDisk's native scanner remains slightly faster and smaller on the synthetic fixture.

**Setup.** Apple M4 Pro, 14 logical CPUs, 24 GiB RAM, macOS 26.5. The current working tree, including its existing UI edits, was frozen before building. Radix was fetched at current upstream `bb98ae2af81f373d31f0c8c4a9c7d94815f3cf19`; OpenDisk remains at `f7551cf21a5c35a963c607401ba5705ec0869b64`. Optimized CLI adapters and development-signed, instrumented Release apps were built outside the measured GitHub directory. Upstream scanner source identity was verified, with only OpenDisk's established cache-directory override in its app build.

There are **72 successful timed observations**: one separate initial run and five rotating warm rounds per engine, layer and scope. Each uses a fresh process and private fresh state for a full scan. No retries, discarded timings or cache purges occurred. All 376 fingerprinted inputs remained unchanged; all 144 power/thermal observations were nominal with Low Power Mode off. The shared sampler's 20 tests and 11 focused normalization/summary assertions passed. Independent review approved the corrected runner and the final evidence.

Warm medians; lower is better. RAM rows are **native CLI process high-water RSS**, not total desktop-app memory.

| Scope and measurement | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| 100,001-entry fixture, app scan + final CPU draw | **78.5 ms** | 90.5 ms | 730.0 ms |
| Fixture, native scan | 45.0 ms | **41.4 ms** | 421.4 ms |
| Fixture, native peak RSS | 29.9 MiB | **23.7 MiB** | 148.0 MiB |
| GitHub folder, app scan + final CPU draw | **4.058 s** | 4.583 s | 6.168 s |
| GitHub folder, native scan | **4.046 s** | 4.610 s | 6.140 s |
| GitHub folder, native peak RSS | **151.7 MiB** | 270.9 MiB | 1,219.6 MiB |

On GitHub, Hahakigami's app time is **11.46% lower than OpenDisk's** and **34.21% lower than Radix's**. Its warm app range, 4.036–4.167 s, does not overlap either comparator's range. Native scanner RSS is 44.01% lower than OpenDisk and 87.56% lower than Radix. On the fixture, Hahakigami's app time is 13.23% lower than OpenDisk, while its native scan is **8.67% slower** and its native peak RSS is **26.02% higher**. This does not isolate a cause for the app/native difference.

**Different default workloads and accounting remain material.** Hahakigami and OpenDisk retain 882,594 root-inclusive GitHub entries; Radix represents 734,346 nodes because it summarizes packages and eligible directories. OpenDisk's app discovery counter excludes the root and reports 882,593; it is preserved without normalization. Hahakigami and Radix both report 175,485,534,208 allocated bytes. OpenDisk reports 197,871,476,736 bytes, a 22,385,942,528-byte difference associated with its lack of full APFS clone deduplication. Fixture allocation is likewise 8,192 / 12,288 / 8,192 bytes. These are stable observations under each app's policies; strict path-set and equivalent-work coverage were not established.

Native timers have engine-specific boundaries. App timers end at the final results CPU Canvas draw, excluding launch and any postdraw cache publication; GPU presentation and interaction latency are unmeasured. Apps retain their default geometry/layout. App-process high-water and sampled descendant RSS remain in the evidence but omit externally parented WebKit helpers, so **no total-app memory advantage is claimed**. OS caches and background desktop load are uncontrolled. Initial runs are not cold-cache measurements. No incremental-rescan or startup-volume claim follows from this cohort. The fixture filesystem rejected invalid UTF-8 names, so the generator substituted a Unicode name.

Portable results with every observation, ratios and binary hashes (`benchmark-results-upstream-2026-10-17.json`, removed). Full raw reports, source archives, build receipts, instrumentation patches and the runner are retained under ignored `benchmark-results/upstream-2026-10-17/`; app binaries remain outside the GitHub workload. All benchmark-owned processes exited. The normal app was not rebuilt, installed or relaunched, and no privacy/keychain settings changed.

## Scan speed versus Radix and OpenDisk on the cleaned scope — 15 October 2026

The current scanner is the fastest of the three on whole GitHub, with the lowest memory. OpenDisk is slightly faster on the 100,001-entry fixture.

**Setup.** The 14 October comparison runner was reused with the same pinned Radix `df1235bfb05acb7a678718166491bcc27817d95d` and OpenDisk `f7551cf21a5c35a963c607401ba5705ec0869b64` adapters and derived apps. Hahakigami uses:
- native: the `f1f360d` release `scan` example
- app: the isolated signed H build, whose scanner is identical except for the non-persisted tail diagnostics

Each engine/layer/scope cell has one separate initial run plus five rotating warm rounds: 72 timed fresh processes in total, full scans, fresh state and default policies. Hahakigami uses the current six-worker desktop Auto bound. There were no cache purges, retries or exclusions. All runs succeeded with zero errors, and fingerprints stayed stable. No untimed path exports were run this time.

Warm medians, with sampled aggregate RSS medians in parentheses:

| Scope/layer | Hahakigami | OpenDisk | Radix |
| --- | ---: | ---: | ---: |
| Fixture, native engine | 139 ms (26 MB) | 135 ms (13 MB) | 496 ms (158 MB) |
| Fixture, packaged app | 210 ms (123 MB) | 172 ms (107 MB) | 796 ms (281 MB) |
| Whole GitHub, native engine | 3,963 ms (165 MB) | 4,437 ms (271 MB) | 5,892 ms (1,267 MB) |
| Whole GitHub, packaged app | 4,062 ms (263 MB) | 4,526 ms (449 MB) | 6,007 ms (1,755 MB) |

**GitHub ratios.** Hahakigami time ratios are 0.893 native / 0.898 app against OpenDisk, and 0.673 / 0.676 against Radix. The ranges don't overlap: the slowest Hahakigami warm GitHub run (4,069 ms app, 4,033 ms native) beats the fastest OpenDisk run (4,484 ms / 4,410 ms).

**Fixture ratios.** Against OpenDisk they are 1.027 native / 1.220 app; against Radix, 0.280 / 0.263.

**The engines do different work, so this is a default-policy workload comparison, not proven equivalent work:**
- **GitHub entry counts:** 868,581 for Hahakigami and OpenDisk, 719,631 for Radix. Radix summarizes packages and eligible directories.
- **Allocated totals:**
  - Hahakigami and Radix both deduplicate APFS clones and match exactly at 170,673,537,024 bytes.
  - OpenDisk does not, and reports 192,843,976,704 bytes.
- **Timers:**
  - Native timers have engine-specific boundaries.
  - App timers end at the final CPU Canvas draw, not GPU presentation.
- **Upstream versions:** Radix upstream `f3a1a44` has seven newer commits, mostly refactors; they were not measured. OpenDisk upstream is unchanged.

Five warm samples on this Mac are not a future or all-hardware guarantee. RSS is 100 ms sampled aggregate, not physical footprint.

**Versus the 14 October comparison** on the larger, uncleaned scope (5,337 / 5,142 / 7,891 ms native), Hahakigami moves from slightly behind OpenDisk to ahead. Part of that comes from the smaller scope, which helps every engine.

See portable evidence with every observation (`benchmark-results-upstream-comparison-summary.json`, removed).

## Six-second gate met after removing stale debug build output — 14 October 2026

**The T3 gate is met by the unchanged H code once the measured scope no longer contains this repository's own stale debug build output.** No scanner behaviour changed. The earlier unmet result stays valid for the larger scope it measured.

**Cause of the tail.** New tail diagnostics show that `Hahakigami/target/debug/deps` had grown to 178,793 entries, mostly about 140k leftover `.rcgu.o` object files from repeated debug test builds. In full GitHub scans this one listing started 1.0–1.8 s in and took 2.9–3.3 s under contention, against about 0.5 s alone. It always finished last while the other five workers idled for 1.0–1.7 s each, so it set the walk end. Every debug build during development added to it: scope nodes grew 1,073,341 → 1,098,162 → 1,101,406 across the day.

**Scope change.** `cargo clean --profile dev` removed 230,886 files (59.3 GiB reported). `~/Documents/GitHub` now has **868,544** entries. Later builds and tests used `CARGO_TARGET_DIR` outside the scope, and the entry count was re-checked unchanged after them. `target/release`, benchmark fixtures and every other file were untouched.

The unchanged H cohort config and binaries were re-run with `scripts/clone_accounting_benchmark.py`: reference (faster pre-listing), latest baseline and H. All use six workers, automatic QoS and full-clone accounting, with one initial plus five rotating warm rounds. There were no retries, exclusions, cache purges or concurrent builds/tests/edits. Results: **72 successful timings, 12 complete exports, 14/14 exact raw comparisons and 666 stable fingerprints.**

| Scope/layer | Reference ms | Baseline ms | H ms | H warm max ms | Baseline / H sampled RSS bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| GitHub native | 4,038 | 3,994 | 3,935 | 3,963 | 177,242,112 / 165,756,928 |
| GitHub packaged app | 4,094 | 4,135 | 4,052 | 4,087 | 278,200,320 / 262,569,984 |
| Flat fixture native | 143 | 142 | 143 | 143 | 26,509,312 / 25,755,648 |
| Flat fixture packaged app | 158 | 159 | 159 | 164 | 103,251,968 / 102,006,784 |

- **Every warm GitHub scan of every role is below 6 s.** H packaged: 4.072, 3.977, 4.052, 4.040, 4.087 s. H native: 3.923, 3.941, 3.919, 3.963, 3.935 s.
- On GitHub, H's median time and sampled RSS are below both controls on both layers.
- One baseline packaged warm run reached 5.065 s; all rows are kept.
- Deductions are identical per scope: **4,096 fixture bytes / 22,170,439,680 GitHub bytes.** The lower GitHub figure reflects clones that lived in the removed build output. Accounting checks are complete with no warnings.

Same-day native pilots of the committed scanner tell the same story. Before cleanup, seven runs took 5.22–6.11 s (median 5.69 s), with five workers idle for 0.7–1.8 s each at the end. After cleanup, six runs took 4.25–4.37 s (median 4.31 s), with at most 0.54 worker-seconds idle in total. The evidence file retains the three diagnostic before-cleanup runs and all six after-cleanup runs.

The planned overlap of clone revalidation with the enumeration tail was not implemented, because almost no idle tail remains to fill.

The new non-persisted `Snapshot.enumeration_tail` reports:
- walk end
- summed final worker waits
- the eight slowest listings: start, duration, entries and worker, with no paths

The `scan` example prints it as `enumeration_tail`. It is never written to archives or caches.

**Keeping the gate met.** The measured scope is live. It regrows whenever builds write into `Hahakigami/target/debug` or into `benchmark-results` inside the scope. Run benchmark-era builds with `CARGO_TARGET_DIR` outside `~/Documents/GitHub`, and record the scope entry count with every cohort.

Five warm samples on this Mac are not a future or all-hardware guarantee. RSS is 100 ms sampled aggregate, not physical footprint. See [verification](VERIFICATION.md) and portable evidence (`benchmark-results-clean-corpus-summary.json`, removed).

## Clone accounting memory and GitHub scan speed — 14 October 2026

**The combined acceptance target remains unmet.** The retained six-worker H candidate improves packaged median time and memory, but one packaged warm scan took **6,147.352708 ms**. The user asked to finish the evidence report; further performance work stopped. The rejected eight-worker policy was reverted. No normal app update was built or installed.

Temporary listing entries and their Result wrappers now fit in 64 bytes instead of 72; node records remain80 bytes. A shared pure sparse allocation plan avoids a dense allocation vector in normal scanner aggregation, and a fresh cursor combines raw reset with attribution in one node-update pass. Public archive callers retain the original dense API. Malformed clone ordering uses the original dense arithmetic before any snapshot mutation. Raw allocation, native paths, full-clone deduction rules, resource forks, cache/archive formats, and cleanup authority remain intact.

Other retained changes release enumeration identity sets only after workers join, reuse bounded owner-path and sparse-metadata cursors with original compatibility fallbacks, and start bulk buffers at 64 KiB, growing to256 KiB only after a validated packed batch. Parsing uses the active slice. Unlimited listing growth changes only above 16,384; capped calibration retains its 4,096 policy. Every required rooted descriptor/file/clone validation remains. Six-worker healthy-host scheduling and all conservative pressure/power/unknown/volume policies are retained.

The H cohort completed **72 successful timings, 12 complete exports, 14 exact raw comparisons and 666 stable fingerprints**. Each cell has a separate initial observation plus five rotating warm rounds, with no retries, exclusions, cache purges or concurrent builds/tests/source edits. Reference is the original faster pre-listing six-worker build; baseline is the latest listing six-worker build. Candidate also uses six. Every role uses full-clone accounting.

| Scope/layer | Faster reference ms | Latest baseline ms | Retained H ms | Latest / H RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,809.000 | 5,572.000 | 5,627.000 | 245,743,616 / 229,048,320 |
| GitHub packaged app | 5,791.590 | 6,005.245 | 5,299.580 | 341,295,104 / 324,976,640 |
| Flat fixture native | 142.000 | 147.000 | 145.000 | 32,489,472 / 31,031,296 |
| Flat fixture packaged app | 158.840 | 160.713 | 159.808 | 105,660,416 / 103,858,176 |

Against the latest build, packaged median improves **11.75%** and sampled aggregate RSS falls **4.78%**, or **16,318,464 bytes**. Both are also below the faster reference in this cohort. Native GitHub median is **0.99% slower** than latest, despite lower native RSS; no all-layer speedup is claimed.

Flat-fixture native app high-water falls **128,483,328→126,730,240 bytes**, a **1.36% reduction**. Its sampled aggregate RSS falls **105,660,416→103,858,176 bytes**. Native CLI RSS falls 32,489,472→31,031,296. GitHub native app high-water falls 344,801,280→327,745,536. RSS/high-water are not physical footprint;100 ms short-run sampling is phase-sensitive.

The five H packaged warm times are **5,696.131208;5,278.762250;6,147.352708;5,259.195667;5,299.580333 ms**. Native warm maximum is 5,979 ms. **19 of 20 candidate warm observations pass; one fails.** This is not a six-second guarantee. A premature commentary claim was corrected before acceptance; the complete rows remain in the evidence.

All timed roles retain deductions **4,096 fixture bytes /22,650,023,936 GitHub bytes**, complete accounting checks and no warnings. Required GitHub validation still covers 100,272 members, with 14,910 bulk and 11,889 direct calls and zero extra initial per-file clone queries. App median validation wall time is 795,792 µs. All14 native raw comparisons match paths, kinds, identities and raw own allocation. App per-node attribution is not independently exported; results are metadata observations, not an atomic snapshot. Candidate app backing geometry passes every timing; this does not prove GPU presentation.

Rejected B–G and I cohorts and all pilots are retained in the portable evidence. Eight-worker I kept every warm scan below 6 s, but packaged median 5,747.199417 ms exceeded both5,480.745333/5,467.406666 controls; sampled RSS333,070,336 exceeded the faster reference 327,565,312. That policy was reverted. Fixed buffer, QoS, worker-count and scheduling experiments were not silently adopted. Profiled scans are excluded from performance results. No arbitrary elapsed-time worker boost was implemented.

**371 Rust tests passed, 22 ignored**, plus 156 frontend and 105 Python tests. Final restored workspace/all-target performance-feature tests, strict Clippy, formatting/diff checks, frontend checking/production build and isolated signed builds passed. The additional three/eight-worker APFS safety test remains; production is H. Independent Max source reviews cleared the retained code after G1/G2 corrections. **T3 performance acceptance remains open; no completion or normal delivery approval is claimed.** Final evidence-only Max review approved report accuracy, while retaining the unmet performance gate. The existing normal app, designated requirement, process state and privacy/keychain settings remain unchanged. See [verification](VERIFICATION.md) and portable evidence with every observation (`benchmark-results-growth-memory-summary.json`, removed).

## Wide-listing scratch and fixture high-water reduction — 14 October 2026

Large bulk listings now reserve output capacity once per fully validated batch, using 1.5× growth beyond 4,096 entries. Small listings keep their previous growth behavior; calibration allowance caps the new reservation. The scanner also reserves child-ID scratch from the known private listing length, capped by the remaining node budget. Neither change touches parsing, directory/file validation, clone checks, cancellation, order, archive/cache formats, scheduling, UI or cleanup authority. Records remain 80 bytes in 4,096-record chunks.

The injected 33,333-entry listing test retains 45,563 output slots instead of 65,536 while preserving every record and the calibration failure boundary. Requested capacity is separate from RSS. A same-binary five-round rotating native fixture pilot measured median high-water RSS **39,288,832→32,096,256 bytes**, with warm times 136–143 ms. All pilot observations are retained; there is no isolated ablation of the two edits.

The final frozen cohort contains **72 successful timings, 12 complete exports, 14 exact raw comparisons and 953 stable fingerprints**. Each native/app scope has a separate initial observation plus five rotating warm rounds, without retries, exclusions, cache purges or concurrent builds/tests/source edits. Reference is the pre-chunk full-clone five-worker build; baseline is the previous chunked/lazy-canvas six-worker build; candidate also uses six workers under the same healthy-host policy. Every role uses full-clone accounting.

| Scope/layer | Pre-chunk ms | Previous ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,500 | 5,366 | 5,430 | 220,905,472 / 221,429,760 |
| GitHub packaged app | 5,528.123 | 5,249.789 | 5,477.248 | 322,404,352 / 323,829,760 |
| Flat fixture native | 144 | 143 | 142 | 39,321,600 / 31,260,672 |
| Flat fixture packaged app | 157.537 | 160.730 | 157.443 | 129,187,840 / 102,629,376 |

**The remaining fixture native app high-water increase is cleared in this cohort:** median falls **135,168,000→128,335,872 bytes**, a **6,832,128-byte (5.05%) reduction**. Every candidate warm observation is below every baseline warm observation: candidate range 128,122,880–133,447,680, baseline 135,020,544–135,905,280 bytes. It is also below the matched pre-chunk high-water median 136,970,240. Native CLI high-water falls 20.50%. Sampled aggregate app RSS falls 20.56%, but its short-run value is sensitive to the process phase caught by the unchanged 100 ms sampler. Native app high-water is the primary regression measure; neither metric is physical footprint.

**Every candidate warm scan remains below six seconds.** GitHub packaged samples are **5.885, 5.770, 4.876, 5.452, 5.477 seconds**; native samples are 5.430, 5.691, 5.287, 5.382, 5.706 seconds. Separate initial candidate observations also pass. Packaged median is 4.33% slower and native median 1.19% slower than the latest baseline; this memory improvement is not a claim of faster large-scope scans. The maximum packaged observation leaves only 115 ms margin. These are measured warm-cache results on this Mac, not future/all-hardware guarantees.

Large-scope packaged savings remain: sampled RSS is **49,627,136 bytes (13.29%) below pre-chunk** 373,456,896, and native app high-water is **49,496,064 bytes (13.18%) below pre-chunk** 375,472,128. Against the latest build, sampled RSS rises 1,425,408 bytes (0.44%) and native app high-water rises 786,432 bytes (0.24%): 325,189,632→325,976,064. This small large-scope increase remains explicit.

All warm native/app deductions match: **4,096 fixture bytes / 22,650,011,648 GitHub bytes**, with complete accounting checks and no warnings. GitHub has 1,075,790 nodes. Required validation still checks 100,272 members with 14,904 bulk + 11,889 direct queries and zero extra initial per-file clone queries. All 14 before/after raw comparisons match native paths, kinds, identities and raw own allocation. App per-node attribution is not independently exported. Every candidate app timing verifies populated map backing geometry; unchanged UI source retains prior frontend/functional smoke evidence without a new interaction-speed claim.

362 Rust tests passed (22 ignored), plus 156 frontend and 105 Python tests, strict workspace/all-target performance-feature Clippy, formatting, framework checking and production build. Independent Max source review found no blockers and required zero corrections. Signed delivery and final evidence review are recorded in [VERIFICATION.md](VERIFICATION.md). Full report: `/private/tmp/hahakigami-listing-memory-20261014/cohort/report.json`; portable evidence with every observation (`benchmark-results-listing-memory-summary.json`, removed). Earlier cohorts remain unchanged below.

## Warm-scan limit and empty-canvas allocation — 14 October 2026

The map now starts with a zero-sized backing store, acquires a context/full-DPR image only for populated layouts, and releases the image when the layout becomes empty. Populated drawing, selection, resize and DPR behavior remain unchanged. Healthy folder scans use six workers only when both hardware and explicitly known active CPU counts are at least 14, thermal state is nominal, and low-power mode is off. Existing conservative/unknown/pressure and volume policies remain unchanged. Clone metadata, descriptor validation, accounting, archive formats, and chunked records are unchanged.

A same-binary three-round rotating native pilot measured five/six/seven-worker medians of 5,260 / 5,107 / 5,194 ms. All nine observations are retained. This informed the six-worker limit; no runtime performance guarantee follows.

The frozen matched cohort completed **72 successful timings, 12 complete native exports, 14 exact raw comparisons and 1,030 stable fingerprints**. Each cell has one initial observation plus five rotating warm rounds. No builds/tests/source edits, cache purges, retries or exclusions occurred during measurement. Reference is the pre-chunk full-clone five-worker build; baseline is the previous chunked five-worker build; candidate adds the changes above. All roles use full-clone accounting.

| Scope/layer | Pre-chunk ms | Previous chunked ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,376 | 5,676 | 5,016 | 222,248,960 /235,536,384 |
| GitHub packaged app | 5,300.947 | 5,424.063 | 5,424.350 | 331,546,624 /328,810,496 |
| Flat fixture native | 142 | 143 | 142 | 39,419,904 /39,403,520 |
| Flat fixture packaged app | 158.491 | 159.235 | 158.672 | 104,595,456 /104,153,088 |

**Every candidate warm scan is below 6 seconds.** GitHub packaged times are **5.463, 5.451, 5.424, 4.667, 5.388 s** (maximum 5.463 s); native times are5.299, 4.699, 5.016, 5.403, 4.650 s. The separate initial candidate observations also pass. The packaged median is effectively unchanged; the baseline packaged warm rows also all pass in this cohort. This establishes the observed limit on this host, not a causal tail improvement or a future/all-hardware guarantee. Native median improves 11.63%, with a 5.98% native RSS increase from the extra concurrency; native RSS remains below the pre-chunk reference.

GitHub packaged sampled peak falls a further **2,736,128 bytes (0.83%)** versus the chunked baseline and **40,714,240 bytes (11.02%)** versus pre-chunk. Native app high-water also falls 333,398,016→332,005,376bytes (pre-chunk371,769,344). Prior packaged savings hold in both measures.

Short-fixture sampled peak falls **442,368 bytes (0.42%)**, but native app high-water rises 135,020,544→135,774,208 bytes (0.56%). Its sampled value is 0.33% above the pre-chunk control 103,809,024bytes. The prior 13 MB sampled increase does not recur in this matched cohort; its cause is unproven. This small mixed result does not establish a consistent all-scope memory reduction. Both measures and every row remain in the evidence; neither measure is physical footprint.

Candidate-only benchmark observations verify expected positive map nodes, CSS dimensions, full-DPR backing dimensions and empty backing release without changing timers, completion gates or 100 ms RSS sampling. All 12 candidate app observations pass. Fixture geometry is 1570×411 pixels at 2× DPR over 785×205.547 CSS pixels. Geometry alone does not prove GPU presentation. A separate post-cohort CUA screenshot showed a populated clone map in the isolated app. All nine non-destructive explorer interaction phases passed. That functional smoke took243,790ms, including233,601ms in scrolling; its cause was not instrumented, and app inspection occurred while waiting. It is not interaction performance evidence or part of the six-second scan cohort.

Every warm native/app role retains identical deductions: **4,096 fixture bytes / 22,650,306,560 GitHub bytes**, with no accounting warnings. GitHub has 1,073,341nodes; required validation still checks 100,272 members with 14,904 bulk + 11,889 direct queries and zero extra initial per-file queries. All before/after native path/kind/identity/raw-own-size comparisons match exactly. App per-node attribution is not independently exported. Source review by `reviewer_high` found no blocking defects and used zero correction rounds; final delivery evidence is tracked in [VERIFICATION.md](VERIFICATION.md).

Full report: `/private/tmp/hahakigami-tail-memory-20261014/cohort/report.json`; portable evidence with all observations (`benchmark-results-tail-memory-summary.json`, removed). Earlier cohorts below remain unchanged historical evidence.

## Measured packaged-memory reduction with chunked records — 14 October 2026

The private node store now allocates records in bounded **4,096-record chunks**, preserving the 80-byte record and all IDs/native data. Appending a later chunk never moves existing full chunks; small first chunks can grow up to the bound, and explicit snapshot cloning remains a deep copy. There is no unsafe/uninitialized storage. Random access stays constant-time. Query cache identity still uses the first owned payload together with completed state, root and node count. Archive/cache formats and every descriptor, identity, clone validation and accounting check are unchanged.

Diagnostics retain `compact_native_v1`/80-byte records and add `record_storage=chunked_4096_v1` plus actual `record_chunks` header storage. Payload and header capacities both contribute to retained totals. The benchmark contract validates existing clone/accounting buffers too; genuine older missing geometry remains compatible. Max-review correction C1 made explicit null/false/list markers fail rather than masquerade as absent historical fields. 347 Rust tests passed (22 ignored), including growth beyond one million records, cross-boundary mutation/native paths/metadata, partial deep-clone append and a 9,000-node v2 archive round-trip with clone owners at 4095/8192. 154 frontend and 104 Python tests passed, with strict Clippy/fmt/check/build and real APFS checks.

The frozen cohort retained **72 successful timings, 12 complete native exports, 910 stable fingerprints and five rotating warm rounds**. Both clone builds used five workers; original reference used three. No builds/tests/edits, cache purges, timed retries or exclusions occurred during measurement.

| Scope/layer | Previous ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: |
| GitHub native | 5,277 | 5,395 | 276,889,600 / 222,625,792 |
| GitHub packaged app | 5,679.778 | 5,509.437 | 373,407,744 / 333,348,864 |
| Flat fixture native | 140 | 144 | 41,762,816 / 39,469,056 |
| Flat fixture packaged app | 160.972 | 160.211 | 105,889,792 / 119,128,064 |

The large-scope packaged sampled peak falls **40,058,880 bytes (10.73%)**, corroborated by native app high-water **375,570,432→336,183,296bytes**, a 39,387,136-byte reduction. Every paired warm round has lower candidate memory in both measures; the native CLI median falls 54,263,808 bytes(19.60%). This is a measured process reduction, beyond buffer-capacity estimates.

Packaged median remains **5.51s** versus 5.68s. Its five times are **5.509, 5.970, 5.457, 5.440, 6.053 s**: four are under 6 s, one is 53 ms above. The median target holds; a strict maximum does not. No row was discarded. Native median is 2.24% slower, with both native medians below6s. Timing variation means neither an all-run nor cross-hardware six-second guarantee is established.

At 1,071,171 nodes, requested record capacity falls **167,772,160→85,852,160bytes**, with 12,288 bytes outer-header capacity included. Total snapshot retained capacity falls **233,571,131→151,663,419bytes**. Live record length remains 85,693,680 bytes. These storage measures are separate from RSS. Required clone work is unchanged: **100,272 members, 14,904 bulk+11,889 direct queries**; validation medians 780,556→801,787µs, grouping requested peak 21,578,264 bytes. Initial extra per-file clone queries remain zero.

Short-fixture sampled app RSS rises 13,238,272 bytes(+12.50%), while native app high-water falls **137,707,520→135,102,464bytes** and native CLI falls 2,293,760 bytes. That mixed short-run result remains explicit; no uniform all-scope RSS gain is claimed. Neither RSS measure is physical footprint.

All 14 raw comparisons match across both scopes, including every repeat and before/after cross-role overlap: native paths/kind/identities/raw allocation are unchanged. Every warm same-model deduction matches **4,096 fixture bytes / 22,650,290,176 GitHub bytes**, with no accounting warnings. App per-node attribution is not independently exported. Full report: `/private/tmp/hahakigami-chunk-memory-20261014/cohort/report.json`; portable evidence: benchmark-results-chunk-memory-summary.json (`benchmark-results-chunk-memory-summary.json`, removed). Results apply to the tested 14-thread ARM Mac; Intel/non-macOS/other hardware were not executed.

## Packaged memory costs and healthy-host scheduling — 14 October 2026

Chart sorting now avoids keys/IDs for zero-selected-size children that were already omitted from the chart. All-positive input keeps the old sort; mixed input uses sparse positive keys, with the old full sort on equal-key collisions to preserve the legacy unstable permutation. Complete map JSON and aggregate boundaries match for both metrics. File lists/search still retain zero-attributed clones. The canvas keeps its backing store when pixel dimensions are unchanged and applies an absolute transform for every redraw, including selection, resize and DPR changes. Initial clone observations move directly to publication; only sparse direct fallbacks use a second queue. Ordering, errors and cancellation-prefix semantics are preserved.

A same-binary three-round rotating worker pilot on this 14-thread host measured native medians 7,272/6,578/5,753ms at 3/4/5 workers. The production folder policy now permits up tofive workers only with explicit known active CPU>=12, enough hardware threads, nominal thermal state and low-power off. Unknown active/thermal evidence and pressure states retain the old policy; volume counts, persisted preferences and QoS behavior are unchanged. This raises the measured candidate from 3 to 5 workers; the speed gain is partly increased parallelism, not solely allocation efficiency.

One Max-review correction tightened the missing-active-CPU case. The initial attempt was stopped and retained as incomplete with 37 timings/6 exports, source and binary artifacts. The corrected frozen cohort contains **72 successful timings, 12 complete native exports, 905 unchanged fingerprints and five rotating warm rounds**. No builds/tests/source edits ran during it; no timed rows were retried/excluded or caches purged in the corrected cohort.

| Scope/layer | Original ms | Previous clone ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,348 | 7,076 | 5,369 | 272,777,216 / 272,187,392 |
| GitHub packaged app | 5,810.574 | 7,293.595 | 5,574.112 | 369,934,336 / 371,752,960 |
| Flat fixture native | 140 | 142 | 141 | 42,647,552 / 40,271,872 |
| Flat fixture packaged app | 205.307 | 160.095 | 158.636 | 105,201,664 / 105,938,944 |

The packaged median reaches **5.57s** on this host (**23.58% faster**); every warm app row is below 6 s. Native time improves 24.12%. Native enumeration wall time falls 5,886,098→4,347,100µs; summed enumeration worker CPU rises 7,069,925→8,164,780µs. Validation falls 980,569→788,095µs, still checking **100,272 members with 14,904 bulk+11,889 direct queries** in both versions. Initial extra per-file clone queries remain zero. Grouping requested-payload peak remains 21,562,096 bytes. Increased concurrency has CPU cost; no energy gain is claimed.

The concrete fixture chart cost falls from **1,066,592 to 72 bytes sort scratch** and **133,340 to 12 bytes cached IDs**, identically in every warm row. This scratch excludes returned/cache IDs and complete query/process peak. Canvas setter-count tests establish backing-store reuse; this is not a measured GPU-footprint total. The metadata-queue test establishes backing-allocation reuse without a second full observation queue.

Whole packaged RSS is near-flat, **not a demonstrated decrease**: sampled fixture RSS rises 737,280bytes(+0.70%), GitHub rises 1,818,624(+0.49%). Native app high-water falls 137,101,312→136,396,800bytes on fixture and rises 372,375,552→374,325,248 on GitHub. The earlier 13 MB sampled fixture gap did not recur; earlier native high-water data had not shown a corresponding clone-induced regression. Both memory measures and all sample arrays are retained. Structural allocations/churn are removed, but total RSS remains variable and is not physical footprint.

All fixture raw comparisons, candidate GitHub repeat, and final GitHub cross-role overlaps match paths/kind/identity/raw-own allocation. Initial overlaps and original/baseline repeats differ only in one `target/debug/.../dep-graph.bin` raw allocation; its cause is uninstrumented and comparisons remain nonmatching. No paths are missing. Every warm same-model deduction matches: **4,096 fixture bytes / 22,650,220,544 GitHub bytes**. There is no independent app per-node attribution export. All actual app workers were 3 previous/reference and 5 candidate. Other hardware/OS performance remains unexecuted.

Full corrected report: `/private/tmp/hahakigami-app-memory-20261014/cohort-corrected/report.json`; incomplete attempt and artifacts beside it. Portable evidence: benchmark-results-app-memory-summary.json (`benchmark-results-app-memory-summary.json`, removed). Earlier cohorts below remain historical observations.

## Validate only groups that can deduct allocation — 14 October 2026

After checking every file for conflicting hard-link aliases, validation classifies `(device, clone ID)` groups by distinct observed inode. A group containing one inode cannot deduct clone data, even when aliases or out-of-scope clones exist. It retains raw allocation and sparse metadata with `verified=false`; it receives no verified owner attribution. Only multi-inode groups enter revalidation, and **every member and hard-link alias** still needs fresh ancestry, identity, size, link and clone checks before subtraction. Failed members reject the entire group; cancellation clears verification. Shared/uncertain baselines remain ineligible for reuse, independently guarded by nonempty clone metadata. Historical archives retain their recorded verification and totals.

Workers reuse active/seen/fallback scratch and one validated basename allocation. Dense groups with at least four wanted files use bulk queries when they cover at least a quarter of the directory; unsupported/omitted fields still use direct fallback. No parser, pure accounting, archive codec or cleanup-authority rules change. The grouping map is released before validation, and retained scratch/group buffers are included in requested-payload diagnostics.

The frozen three-way cohort completed **72/72 timings, 12 complete native exports and 903 stable fingerprints**, with five rotating warm rounds plus separate initial observations. No builds/tests/source edits ran during it; no samples were retried/excluded or caches purged.

| Scope/layer | Original ms | Previous clone ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,381 | 9,221 | 7,392 | 286,162,944 / 272,842,752 |
| GitHub packaged app | 5,717.948 | 9,215.936 | 7,416.271 | 380,239,872 / 372,850,688 |
| Flat fixture native | 139 | 142 | 141 | 41,811,968 / 41,664,512 |
| Flat fixture packaged app | 229.989 | 176.701 | 177.824 | 103,890,944 / 116,965,376 |

GitHub time improves **19.84% native / 19.53% app**. Native validation wall time falls **3,090,894 → 1,043,740 µs**, and summed worker time falls **9,236,340 → 3,112,242 µs**. Revalidated members fall **406,967 → 100,336**; parent walks **35,339 → 14,257**. Final queries change from **10,972 bulk + 92,940 direct** to **14,920 bulk + 11,891 direct**. Initial extra per-file clone queries remain zero. Requested grouping peak falls **30,948,136 → 21,510,952 bytes**. The six-second target remains unmet; observations are metadata scans, not atomic snapshots or reclaimed-space estimates.

The native flat-fixture memory fix holds (also below original RSS 42,729,472). Packaged-fixture RSS is **13,074,432 bytes higher** in this cohort; it is not omitted. A separate ten-observation paired diagnostic using the same 100 ms sampler gives medians **136,347,648 previous / 135,249,920 candidate**, with wide ranges 103–138MB for both roles. Both datasets are retained. This does not establish a reproducible app-fixture increase or improvement, and its cause is unresolved. These are sampled/high-water RSS measures, not physical footprint.

All fixture raw comparisons and final GitHub cross-role overlaps match paths, kinds, identities and raw own allocation exactly. Earlier GitHub overlaps show 15 build-artifact raw allocation differences; reference/baseline repeats show 16 and candidate repeat showsone, all under `Hahakigami/target/debug`. No missing paths or kind/identity mismatches occur. Their allocation drift was not instrumented; those comparisons remain nonmatching. Every warm same-model native/app deduction is identical: **4,096 fixture bytes / 22,651,383,808 GitHub bytes**. App per-node attribution is not independently exported.

One Max-review portability correction followed cohort closure: the private `Raw` status is now macOS-only. The measured source snapshot and final source hashes are recorded separately. The rebuilt macOS scanner executable is **byte-identical** to the measured binary; 33 native clone tests and strict workspace Clippy re-passed. Only the macOS/ARM target is installed, so full non-macOS and Intel builds remain unexecuted. Full report: `/private/tmp/hahakigami-clone-validation-20261014/cohort/report.json`; paired diagnostic beside it. Portable evidence: benchmark-results-clone-validation-summary.json (`benchmark-results-clone-validation-summary.json`, removed). Earlier cohorts below remain historical observations.

## Sparse clone listings and balanced validation — 14 October 2026

This follow-up removes inline clone metadata from every temporary listing entry: `Entry`/`Result<Entry>` returns from 104 to 72 bytes, with shared observations carried in a sparse ordinal sidecar. The compact node remains 80 bytes. Rich-listing retries discard their private sidecar; malformed fallback retains raw accounting and separate warnings. Borrowed native names avoid an extra copy, and each scan worker retains one bounded 256 KiB enumeration buffer. A dirty-tail regression verifies that parsing still respects reported record boundaries.

Final checks use whole-parent tasks with the original three-worker bound. Large query workloads start first; smaller tasks retain their original order. Every task freshly checks root/ancestry, every participating file still needs matching metadata, and cancellation or a bad member rejects the entire affected clone group. Resource forks, hard links, raw sizes, archive/cache rules and cleanup authority are unchanged.

The new frozen comparison retained **72/72 successful timings, 12 complete native exports, 902 stable fingerprints and five rotating warm rounds**. Builds/tests completed before measurement; no timed retries, exclusions or cache purges were used.

| Scope/layer | Original ms | Previous clone ms | Candidate ms | Previous / candidate RSS bytes |
| --- | ---: | ---: | ---: | ---: |
| GitHub native | 5,380 | 9,453 | 9,152 | 288,980,992 / 278,118,400 |
| GitHub packaged app | 5,853.350 | 9,349.172 | 9,258.867 | 380,174,336 / 378,404,864 |
| Flat fixture native | 143 | 143 | 143 | 49,659,904 / 41,844,736 |
| Flat fixture packaged app | 210.100 | 168.707 | 165.064 | 105,660,416 / 105,218,048 |

The flat-fixture native regression is removed: candidate RSS is also below the original 42,795,008 bytes. GitHub native time improves 3.18% and RSS 3.76%; packaged-app timing changes by only 0.97%, insufficient evidence of a meaningful app speedup. **The six-second target is not reached.** Clone revalidation still costs a median 3,046,743 µs wall time (previous 3,237,363), checking all 406,112 members. Candidate final queries are 10,970 bulk plus 92,941 direct; previous direct median is 102,765 (worker-order-dependent ranges retained). Summed worker validation time increases from 7,912,154 to 9,102,552 µs despite the lower wall time. Requested grouping peak remains 30,925,760 bytes. Initial extra per-file clone queries remain zero.

All fixture raw comparisons and final GitHub cross-role overlaps match paths, kind, identities and own sizes exactly. Earlier GitHub exports differ in 11 raw allocations in `Hahakigami/target/debug`; original/reference repeat exports also change, so these observations do not establish initial raw parity. Their physical-allocation drift was not instrumented or reclassified as equivalent. Repeated exports also capture a `devices.json` replacement and a log-size update in another active repository. No paths are missing. Every warm same-model native/app deduction matches: 4,096 fixture bytes and 22,651,359,232 GitHub bytes. There is no independent packaged-app per-node attribution export.

RSS is high-water or sampled aggregate RSS, not physical footprint. The prior 6 MiB listing-capacity explanation was a source-derived estimate; this cohort supplies the actual regression check. Full private artifacts: `/private/tmp/hahakigami-clone-next-20261014/cohort/report.json`. Portable evidence: benchmark-results-clone-listing-summary.json (`benchmark-results-clone-listing-summary.json`, removed). Earlier cohorts below remain historical observations with different live filesystem state.

## APFS clone-accounting optimization — 14 October 2026

The follow-up preserves full-clone data-fork deductions, resource forks, native paths, raw allocation and cleanup safeguards. Rich descriptor-relative bulk enumeration supplies initial clone attributes together with raw metadata. Final validation uses the existing scan worker count, with disjoint private status slices and whole-group rejection after joining. Dense parent listings use checked streaming bulk attributes; small/sparse groups retain individual `getattrlistat` checks. Every participating file still needs matching identity, type, link state, sizes and clone evidence. Each worker reuses a 256 KiB buffer and a bounded ancestor-descriptor chain, freshly checking the held root and every cached edge on parent switches. No contents or cloud data are opened.

Sparse preliminary probe results keep listing ordinals without copying bulk metadata. One pure accounting plan supplies totals, deductions and inspector attribution; archive validation uses the same implementation. Alias maps are released before group construction. The 80-byte node record, accounting model, archive/cache rules and action authority remain unchanged. Malformed initial rich metadata triggers one independently reopened raw listing, with a separate accounting warning and no clone evidence from that listing; earlier permanent enumeration errors remain visible.

The frozen three-way cohort completed **72 successful observations and 12 complete native exports**, with one separate initial observation and **five rotating warm rounds** per role/layer/scope. Sources, binaries, full packaged apps and harness fingerprints stayed unchanged. No samples were retried/discarded or caches purged. Original reference is hard-link-only commit `3d178a5ec8b79f0194ca267dfb0d5169d363f252`; preceding clone binaries/sources are retained separately from the optimized candidate.

| Scope/layer | Original median ms | Previous clone median ms | Optimized median ms |
| --- | ---: | ---: | ---: |
| 100,001-entry fixture/native | 46 | 113 | 49 |
| Fixture/packaged app | 117.598 | 139.986 | 74.620 |
| GitHub/native | 5,668 | 21,408 | 9,852 |
| GitHub/packaged app | 6,075.971 | 21,135. 333 | 9,901.727 |

| Scope/layer | Original RSS bytes | Previous clone RSS bytes | Optimized RSS bytes |
| --- | ---: | ---: | ---: |
| Fixture/native | 42,074,112 | 43,057,152 | 49,594,368 |
| Fixture/packaged app | 131,629,056 | 117,309,440 | 141,672,448 |
| GitHub/native | 214,941,696 | 341,934,080 | 289,800,192 |
| GitHub/packaged app | 344,948,736 | 420,315,136 | 388,743,168 |

GitHub time decreased **54.0% native / 53.1% packaged** against the preceding implementation; median RSS decreased **15.2% / 7.5%**. It remains **1.74× / 1.63×** original scan time and **34.8% / 12.7%** above original RSS. Flat-fixture RSS increased, so this is not a uniform memory win. Native RSS is process high-water; packaged RSS is the owned-process/verified-descendant aggregate sampled every 100 ms, not physical footprint. Native and final CPU-draw timers have distinct boundaries; no GPU-presentation, cold-cache, Home/startup or external-app claim is made.

Every same-model warm observation retained exactly **4,096 fixture / 22,644,379,648 GitHub bytes** of clone deductions. The original reference intentionally counts that shared data again. Fixture repeats/overlaps match all raw fields. Initial GitHub overlap also matches every native path, kind, identity and raw own size. Later GitHub exports captured live work in an unrelated repository: 8–9 additions, one identity change; final overlaps have one additional late path. Those later comparisons are retained as nonmatching, not reclassified as exact parity or discarded. All common-path raw allocated fields match. Independent packaged per-node attribution is not exported; native regressions verify owner/deduction parity, including worker order, resource forks and cross-worker group failure.

GitHub native warm medians removed **882,414 additional initial per-file queries**, obtaining those fields in enumeration instead. Final validation still attempts **404,838 participating files**, using about **10,966 rich bulk calls plus 101,748 direct queries** rather than 404,838 individual calls. Three workers reduce validation wall time to approximately **3.55 s**; summed worker query/ancestry time is approximately **8.14 s** and is not serial elapsed time. Largest grouping payload bound falls from **56,563,696 to 30,871,920 bytes**. The payload bound includes retained workspaces and conservative summed worker peaks, excluding hash controls, reusable path scratch and allocator/RSS overhead. Exact medians and all samples/counters are in the summary.

Portable optimization summary (`benchmark-results-clone-optimization-summary.json`, removed). Full report, raw exports, source snapshots and retained binaries are under `/private/tmp/hahakigami-clone-optimize-20261014/`. The signed normal app was built separately after measurements; this does not restart a user app or modify privacy grants.

## Historical first full-clone accounting cost — 14 October 2026

The first delivered candidate automatically reconciles hard links, then deduplicates verified full-clone **data forks** across distinct inodes. Raw own allocation remains available; resource-fork allocation stays per inode. Initial metadata probes run outside the snapshot write lock. Final validation holds at most one cached parent descriptor, reopens its ancestor chain when switching parents, and still checks every participating file's identity, link count, sizes and clone attributes. Shared/uncertain baselines require full scans.

The final frozen cohort used a fresh process and private state per full scan, one separate initial observation plus **five rotating warm rounds** per baseline/candidate cell: **48 timed observations and eight untimed native exports**. All timed runs succeeded; source/binary fingerprints stayed stable. All eight repeat/overlap comparisons matched exact native paths, identities, kinds and raw own sizes with complete coverage. Declared accounting policies differ intentionally: raw-field matches do not mean attributed totals should match. Native exports do not establish independent packaged-app path parity. No OS caches were purged, samples discarded, thresholds relaxed or earlier candidate samples reused.

| Scope/layer | Baseline median ms | Clone-aware median ms | Time ratio | Baseline RSS bytes | Clone-aware RSS bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100,001-entry fixture/native | 45 | 114 | 2.533 | 41,631,744 | 43,548,672 |
| Fixture/packaged app | 114.669 | 141.977 | 1.238 | 135,626,752 | 123,977,728 |
| GitHub/native | 5,395 | 20,840 | 3.863 | 175,177,728 | 300,122,112 |
| GitHub/packaged app | 5,828.656 | 20,798.528 | 3.568 | 295,698,432 | 388,300,800 |

Native RSS is process high-water; packaged RSS is the owned-process/verified-descendant aggregate sampled every 100 ms, not physical footprint. Native timing is the core scanner/aggregation timer; app timing includes the native pipeline and final CPU Canvas draw, excluding launch and GPU presentation. Normal policies used three workers on this host. These are warm-cache observations on one Mac and two scopes, with no Home/startup or external-app claim.

The GitHub tree retained **1,046,304 entries** in both candidates. Verified clone attribution removed **22,643,912,704 bytes** of duplicate data-fork allocation: **255,723,397,120 → 233,079,484,416** attributed bytes. The fixture removed exactly 4,096 bytes: 12,288 → 8,192, preserving all paths. These differences do not estimate reclaimed space.

GitHub native medians recorded 874,537 initial file queries, 403,547 file revalidation attempts and 35,995 parent-walk attempts. Initial metadata time sums concurrent workers (**7,636.402 ms**) and must not be added as serial wall time; final revalidation took **10,886.802 ms**. Grouping's largest requested-capacity/payload estimate was **56,512,056 bytes**, excluding hash controls, path scratch, allocator overhead and RSS. The 80-byte node record remains unchanged, with additional sparse clone metadata/attribution buffers.

**The accounting change costs substantial time and memory:** GitHub native scans are 3.86× slower with 71.3% more peak RSS; packaged scans are 3.57× slower with 31.3% more sampled RSS. Automatic accounting was explicitly chosen despite additional query cost. No speed improvement or prior compact-memory acceptance is claimed for this new workload.

A complete earlier candidate cohort is retained: 48 successful timed rows, eight exports and stable fingerprints, with GitHub medians **38,748 ms native / 39,021.493 ms app**. Counters identified approximately 29 seconds of repeated ancestor walking. The bounded parent-handle optimization reduced that work without skipping file checks; its replacement/symlink/switch-back regressions and independent Max review passed. The first cohort had two raw allocation-field changes in the baseline repeat/before overlap, with unchanged paths, and identities; the cause is unproven. All final-cohort raw comparisons passed. Both cohorts and source/binary snapshots are retained, not combined.

Portable aggregates, initial/warm samples, counters and executable hashes: clone-accounting summary (`benchmark-results-clone-accounting-summary.json`, removed). Full local reports/exports: `/private/tmp/hahakigami-clones-20261014/cohort-optimized/`; earlier cohort: `cohort-final/`; original candidate sources/binaries: `unoptimized-candidate/`. Reproduce with `scripts/clone_accounting_benchmark.py` using frozen engine CLI, isolated app and native-exporter executables, explicit absolute scopes and an output directory outside them.

## Historical compact-tree result before clone accounting — 14 October 2026

**Acceptance achieved for the fixture/GitHub compact-tree cohort: all 14 gates pass.** Against baseline `b8bd05947ab6f0c994cc50354eb679a44df38859`, GitHub full-scan median RSS fell **58.0% native and 49.3% signed app**, and retained snapshot capacity fell **61.6%**. Every timing ratio is below 1.05 (largest 1.041879). These results do not replace the older Home/startup result below.

The final frozen cohort used one separate initial and **15 consecutive rotating warm rounds** per cell: 448 timed observations, followed by 64 untimed raw exports and 60 exact SQL comparisons. All exports were complete and equivalent within/across layers and modes; source/binary fingerprints stayed unchanged (262 entries). Every requested incremental timed run/export was actually incremental with reused entries and no fallback. No sample was discarded, phases subtracted, thresholds relaxed or prior-attempt candidate rows reused. The report began 2026-10-13 UTC; documentation date is 2026-10-14 Europe/Kiev.

| Scope | Layer | Mode | Baseline ms | Candidate ms | Ratio |
| --- | --- | --- | ---: | ---: | ---: |
| fixture | native | full | 193.654 | 192.266 | 0.992836 |
| fixture | native | same_process | 84.952 | 80.478 | 0.947331 |
| fixture | native | persisted | 88.173 | 88.229 | 1.000636 |
| fixture | native | changed | 206.910 | 195.957 | 0.947062 |
| fixture | app | full | 201.902 | 203.447 | 1.007654 |
| fixture | app | same_process | 85.961 | 83.468 | 0.971005 |
| fixture | app | persisted | 101.428 | 98.350 | 0.969652 |
| fixture | app | changed | 217.563 | 194.083 | 0.892074 |
| github | native | full | 5693.951 | 5727.656 | 1.005919 |
| github | native | same_process | 1801.877 | 1859.203 | 1.031815 |
| github | native | persisted | 1994.847 | 2063.372 | 1.034351 |
| github | app | full | 5715.933 | 5727.134 | 1.001960 |
| github | app | same_process | 1785.178 | 1858.075 | 1.040834 |
| github | app | persisted | 1999.970 | 2083.727 | 1.041879 |

The owned fixture has 100,001 entries, 16,384 allocated bytes. GitHub has 1,029,239 entries, 253,189,095,424 allocated bytes; all observations have zero errors/exclusions. Actual workers are 3 for fixture modes and GitHub full, 4 for GitHub same-process/persisted; paired scheduling is unchanged. Build-created scope differences from older attempts are retained rather than combined.

| GitHub full memory measure | Baseline bytes | Candidate bytes | Ratio |
| --- | ---: | ---: | ---: |
| Native process high-water RSS | 478,855,168 | 201,228,288 | 0.420228 |
| Signed app sampled aggregate RSS | 561,692,672 | 284,508,160 | 0.506519 |
| Retained snapshot requested capacity, both layers | 326,839,155 | 125,567,485 | 0.384187 |

RSS includes runtime/process overhead; app sampling is every 100 ms and excludes unrelated helpers. Retained capacity excludes allocator headers, scan/query scratch and IPC. Candidate GitHub capacity includes 83,886,080 record bytes, 35,651,584 native-name bytes, 6,029,312 child bytes, 32 root bytes and 477 worker bytes; path/display overrides and issues are zero. Records are 80 bytes versus 144 before; full identity and all four u64 sizes remain. Root is stored once. Memory targets apply to GitHub; fixture app full RSS ratio is 0.996444 and is not memory-gated.

The final bounded correction materializes map display-name `Cow` keys once per sort, preserving the original selected-size/display-name comparator and equal-key ordering. Diagnostic profiles showed repeated UTF-8 validation in map sorting, prolonging the `session.issues` mutex held by `get_map`; `list_page` waited on that issue-cache mutex. Lock policy is unchanged. Temporary keys are separately reported: fixture largest key-vector sample 1,066,592 bytes, GitHub 23,168 bytes, both with zero owned lossy-name capacity. Keys are discarded before return; these are one-sort capacity samples, not retained memory, concurrent-query peak or RSS. Persistent caches retain IDs.

Earlier attempts remain evidence: attempt 1 failed fixture app/full (1.051387); attempt 2 failed native fixture/persisted and had baseline untimed incremental fallbacks, making coverage inconclusive; the predeclared 15-round attempt 3 still failed fixture app/full (1.055708). Broader rounds addressed observed bimodal journal replay, without exclusions. The final fresh 15-round cohort follows the profiled correction; it is not an unchanged-candidate retry. Full reports and exact failed-source/binary evidence remain local.

| Store design from pinned primary source | Path/name/topology strategy | Comparison limit |
| --- | --- | --- |
| [Radix FileTreeStore.swift](https://github.com/colinvkim/Radix/blob/df1235bfb05acb7a678718166491bcc27817d95d/Radix/Models/FileTreeStore.swift) | Flat records with URL/name values and pooled child topology | Representation reference; this cohort does not establish upstream performance parity |
| [OpenDisk FileTree.swift](https://github.com/137137137/OpenDisk/blob/f7551cf21a5c35a963c607401ba5705ec0869b64/OpenDisk/Services/Scanning/FileTree.swift) | Basename plus parent/first-child/next-sibling reconstruction; identity largely for hard links | Different metadata/safety contract |
| Hahakigami compact tree | Native basename/child arenas, root once, checked parent-path reconstruction, full identity/all four size fields | Held-root/no-follow/fresh identity/alias/journal safeguards retained |

Legacy V1 JSON and HAHCAC02 archive compatibility remain verified; frontend `NodeView` is unchanged. Final native checks: 288 passed, 22 opt-in ignored; strict Clippy default/feature, formatting, frontend 139 tests/check/Vite build and Python 99 tests passed. Independent Max reviews approved compact core/native scope and the exact final correction. Final isolated signed-app interaction smoke verified all nine phases. The normal app was built with `pnpm desktop:build`, deep/strict signature verified and its designated requirement matched exactly; it was never launched, and no privacy grants/session state were changed.

Portable aggregates and fingerprints: compact summary (`benchmark-results-compact-memory-summary.json`, removed). Full local archive: `/private/tmp/hahakigami-compact-memory-20261013-275cafc3/attempt-4-final-15/`. Warm OS caches, one Mac and these scopes limit generalization; no cold-cache, Home/startup or external-app acceptance claim follows.

## Historical Home/startup memory target

The earlier target was at least 20% lower median peak RSS on Home and startup `/` full scans, separately for the native pipeline and packaged app, with no more than 5% timing regression against the frozen Hahakigami baseline. Rescans also have a 5% timing limit. One initial round stays separate from five rotating warm rounds. External comparisons require reconciled paths and accounting.

**Historical Home/startup result: acceptance not achieved.** The corrected cohort completed 240 successful timing runs, with all 136 source/binary fingerprints unchanged. Home raw memory reductions were 20.36% native and 23.77% packaged; startup reductions were 16.76% and 16.80%, below target. Every timing median stayed within 5%. Fixture coverage passed both layers; Home/startup coverage is inconclusive. See [corrected results](#corrected-performance-and-memory-results) and the report (`benchmark-results/performance/suite-corrected-report.json`, local only).

The portable result summary (`benchmark-results-memory-summary.json`, removed) is checked in with aggregate measurements and binary hashes. Links into `../benchmark-results/performance/` refer to local ignored raw reports and build artifacts, which are retained on the measuring Mac and are not published with the repository.

The earlier OpenDisk timing study below is historical. It used a different baseline, worker policy and three warm rounds; its scores do not establish the current memory target or equivalent external-app coverage.

## Reproduce

Run from the repository on macOS 15 or newer with a Rust toolchain and Xcode command-line tools. These scripts inherit the shell’s existing access. They never grant Full Disk Access, purge caches, or read file contents during scanning.

```sh
cargo build --release -p disk-scan --example scan
python3 scripts/build_opendisk_benchmark.py
python3 scripts/benchmark.py --output benchmark-results/comparison.json
python3 scripts/worker_sweep.py "$HOME/Documents/GitHub" --output benchmark-results/workers.json
python3 scripts/compare_fixture.py --output benchmark-results/fixture.json
python3 scripts/scan_trial.py "$HOME" --workers 8 --output benchmark-results/trial.json
python3 -m unittest discover -s scripts -p 'test_*.py'
```

OpenDisk is pinned to commit `f7551cf21a5c35a963c607401ba5705ec0869b64`. The build script clones into ignored `benchmark-results/.cache/`, checks the exact commit and clean checkout, and compiles its unmodified `ScanEngine`, traversal, bulk reader, tree, metrics, volume and partial-result code with `swiftc -O -swift-version 6` targeting host architecture/macOS 15. A benchmark-only replacement for `ScanCache` returns `nil` from `peek/load` and does nothing in `save`; it never touches OpenDisk’s user cache. Its adapted API declarations carry MIT attribution and the upstream license in `scripts/OPENDISK-LICENSE.txt`. The reference source itself remains unmodified.

The comparison runs a separate process for each complete scan: an initial Hahakigami/OpenDisk pair, then three alternating warm pairs per scope. Initial runs are recorded separately and are **not** described as cold-cache scans. OS filesystem caches remain intact. `--scope NAME=PATH` can be repeated; `--opendisk-mode traversal` uses the original traversal default (the root’s own device only). `--opendisk-mode system-data --scope startup=/` explicitly allows the System and Data devices, matching the original engine’s primary traversal device policy while omitting its sibling-volume composition. Both modes are diagnostic and are not the app-performance comparator. Hahakigami additionally includes otherwise-unrepresented Data-only branches, so System/Data mode does not automatically certify equivalent path coverage. Each process has a 600-second timeout, configurable with `--timeout`.

Results include scanner elapsed time, process wall time, entry counts, allocated totals, issue counts, and process peak RSS in bytes. Rust issues include enumeration failures, disappearing entries, and intentionally excluded mounts; OpenDisk reports unreadable directories. These counts are not semantically identical, and their mismatch blocks the strict aggregate gate. Measurements are buffered until all timed scans finish, avoiding checkpoint writes into the scanned tree. OpenDisk reachable entries include the root; stored nodes are reported separately because root composition may leave unreachable nodes.

## Gate and limitations

`matching_aggregate_coverage` requires every warm pair to complete and exactly match entries, allocated bytes and unreadable counts. `raw_speed_target_met` reports only the median ratio; `equivalent_coverage_speed_gate` also requires matching aggregate coverage. Path-set verification and a direct built-app comparison remain separate requirements, so the harness always records overall `acceptance_met: false`. Equal aggregate totals alone do not prove equal path sets.

OpenDisk’s `/` engine scans the boot System/Data group and merges mounted sibling system volumes. Hahakigami’s startup scan excludes unrelated mounts and avoids mirrored Data traversal. These intentional policies can cause a root coverage mismatch. Filesystem activity, unsupported metadata, permission behavior and unreadable-directory reporting can also differ. A mismatch is a blocked acceptance result, not permission to compare a smaller scan as though it were equivalent.

The worker sweep rotates 1/2/4/8 order over three rounds. It names the fastest measured worker count only when all scans completed and entry/allocated/error totals stayed identical. Results are specific to that dataset and host, not a universal concurrency optimum.

## Recorded result

The unchanged baseline is recorded in `benchmark-results-comparison.json` (`benchmark-results-comparison.json`, removed), beginning 10 October 2026 at 19:54 Kyiv time. Host: Mac16,8, arm64, 14 logical CPUs, 24 GiB RAM, macOS 26.5; Rust 1.99.0 and Swift 6.3.3. Binary and scanner source SHA-256 hashes identify the measured candidate, whose working tree had no commit yet. Both scanners inherited the same existing shell permissions; privacy settings were not changed.

| Scope | Engine | Initial scan | Three warm scans | Warm median | Maximum peak RSS |
| --- | --- | ---: | --- | ---: | ---: |
| Home | Hahakigami, 8 workers | 53.411 s | 67.882 / 56.363 / 54.174 s | **56.363 s** | 1.42 GiB |
| Home | OpenDisk, 3 workers | 18.912 s | 19.301 / 19.314 / 19.204 s | **19.301 s** | 0.92 GiB |
| Startup | Hahakigami, 8 workers | 77.909 s | 75.852 / 77.825 / 76.328 s | **76.328 s** | 1.76 GiB |
| Startup | OpenDisk, 7 workers | 18.715 s | 20.263 / 18.165 / 18.098 s | **18.165 s** | 0.84 GiB |

Hahakigami was **2.920× slower on Home** and **4.202× slower on startup**. Neither raw speed target passed. Neither aggregate coverage gate passed. This is a failed baseline, not a claim that the performance requirement was met.

Home warm entry counts were 2,991,192–2,991,251 for Hahakigami and 2,991,251–2,991,252 for OpenDisk; issue counts were 135–139 versus 134 unreadable directories. Final Home allocated totals were 596,018,880,512 versus 596,018,814,976 bytes. The small live-entry and byte changes prevent exact aggregate equivalence even on this largely comparable subtree.

Startup warm entry counts were 4,093,456–4,093,500 versus 4,114,091–4,114,138; issue counts were 449 versus 441 unreadable directories. Final allocated totals were 711,664,422,912 versus 746,272,612,352 bytes. OpenDisk’s additional sibling-volume policy accounts for a material scope difference; this report does not certify a complete path-level reconciliation.

Full Disk Access remains a manual setup step. Inaccessible entries, intentional mount exclusions, and live filesystem changes are retained in the results. Native GUI launch, maps and disposable Trash checks are separate validation; a direct OpenDisk.app performance comparison and equivalent path-set coverage remain outstanding.

The scanner owner is now optimizing traversal/aggregation against this frozen baseline. New measurements must carry new binary/source fingerprints and be reported separately rather than replacing this failure.

## Optimized trial and concurrency selection

The first optimized eight-worker Home trial completed in **14.088 seconds**, with a 14.550-second process wall time and 1.25 GiB peak RSS. This single pre-root-anchor-correction measurement is recorded in `benchmark-results-optimized-home-trial.json` (`benchmark-results-optimized-home-trial.json`, removed); it is not the final comparison.

Three rotated rounds on the GitHub dataset selected **eight workers**. All twelve scans had identical entry, allocated and issue totals. Medians were **10.590 / 5.625 / 3.160 / 2.345 seconds** for 1 / 2 / 4 / 8 workers. `benchmark-results-workers.json` (`benchmark-results-workers.json`, removed) preserves every measurement and candidate fingerprints. The final root-anchor correction adds a one-time startup guard without changing the selected concurrency.

The final reviewed root-anchor comparison uses new fingerprints and is reported below. Failed coverage/app-level gates remain explicit.

The final root-anchor CLI and OpenDisk engine matched exact entry, allocated-byte and issue totals on all five deterministic fixtures: regular files, symlinks (including a loop and broken target), hard links, Unicode/newline/spaced names, and a sparse file. `benchmark-results-fixtures.json` (`benchmark-results-fixtures.json`, removed) records both engines' raw outputs. These fixtures support compatible basic counting semantics; they do not prove equivalent full-disk path coverage or eliminate live-file, permission and unsupported-metadata differences.

## Final reviewed comparison

`benchmark-results-optimized-comparison.json` (`benchmark-results-optimized-comparison.json`, removed) records the final release CLI after independent root-handoff/target-selection review, build, formatting, Clippy, and regression checks. The benchmark explicitly uses eight workers. Both scanners ran an initial scan and three alternating warm full scans on each scope; no compilers or separate scans ran during measurement.

| Scope | Engine | Initial scan | Three warm scans | Warm median | Maximum peak RSS |
| --- | --- | ---: | --- | ---: | ---: |
| Home | Hahakigami, 8 workers | 12.165 s | 11.752 / 11.791 / 11.542 s | **11.752 s** | 1.25 GiB |
| Home | OpenDisk, 3 workers | 19.136 s | 18.793 / 19.376 / 18.953 s | **18.953 s** | 0.89 GiB |
| Startup | Hahakigami, 8 workers | 19.245 s | 19.646 / 19.462 / 19.495 s | **19.495 s** | 1.68 GiB |
| Startup | OpenDisk, 7 workers | 18.226 s | 18.376 / 18.554 / 18.489 s | **18.489 s** | 0.86 GiB |

The optimization cuts Hahakigami's baseline Home median by 79.1% and startup median by 74.5%. Home now beats the reference engine by 38.0%; startup remains 5.44% slower. The overall planned performance acceptance is **not met**.

Home entry and issue counts match in every paired run: 2,995,937 entries in the first two warm pairs and 2,995,936 in the third, with 134 issues/unreadable directories throughout. Allocated totals differ by 3,141,632 bytes in the first warm pair and 20,480 bytes in the second; the third pair matches exactly at 596,824,551,424 bytes. Live allocated-byte changes therefore fail the deliberately strict all-pairs aggregate gate despite compatible basic counting and the speed win. This report does not attribute all drift to a particular background process or certify equivalent path sets.

Startup warm Rust entries are 4,099,204 / 4,099,204 / 4,099,271, versus 4,119,841 / 4,119,842 / 4,119,908. Rust reports 448 issues, while OpenDisk reports 441 unreadable directories. Final allocated totals are 712,505,241,600 versus 747,113,168,896 bytes. The sibling-volume composition/exclusion difference remains material.

### System/Data diagnostic

`benchmark-results-system-data.json` (`benchmark-results-system-data.json`, removed) uses the unmodified OpenDisk traversal with explicit System and Data device IDs, without the original engine's sibling-volume composition or periodic partial-result assembly. It is a diagnostic, not an OpenDisk.app benchmark. Its warm median is **18.051 s**, versus Hahakigami's **19.613 s**, an **8.65% gap**. Entry differences fall to tens rather than roughly 20,600; policies for Data-only branches, `/Volumes`, unreadable/excluded mounts and live mutations still prevent a verified path match. All strict acceptance flags remain false.

### Separate stack profile

```sh
python3 scripts/profile_scan.py / --workers 8 --seconds 5 \
  --sample-file benchmark-results/root-profile.txt \
  --output benchmark-results/root-profile.json
```

`benchmark-results-root-profile.json` (`benchmark-results-root-profile.json`, removed) records a successful five-second, one-millisecond-interval `sample` of the exact scanner PID spawned by the script. It uses no sudo and changes no limits, I/O policy, or privacy settings. The raw call graph stays in ignored `benchmark-results/root-profile.txt`; profiled elapsed time is explicitly excluded from benchmark evidence. CLI syntax was verified against the installed `/usr/bin/sample -h` after Context7 returned no relevant profiler documentation.

Collapsed leaf observations were `getattrlistbulk` **16,004**, `__openat` **8,329**, `fstatat` **1,804**, `madvise` **335**, tiny malloc **93**, contended RwLock **31**, and memset **5**. Openat stacks name `descriptor::open_child`; no named absolute-reopen fallback appeared, although inlining prevents proving it never occurred. These are stack residency observations, including time blocked in syscalls, **not CPU percentages**. Metadata/syscall work dominates this sampled interval; large buffer clearing or lock contention is not established as the main bottleneck.

A sibling Python process in the same exec-shell configuration reported `RLIMIT_NOFILE` soft **1,048,575**, hard unlimited. Reviewed scanner source clamps retained descriptors to 512 after a 64-descriptor reserve, so a hypothetical inherited soft limit of 256 does not explain this benchmark. This is same-environment evidence, not a direct resource-limit query of the exited child. A future profile script records the parent limit immediately before spawning.

The next optimization decision should use this evidence and preserve metadata completeness/root authority. No post-profile code changes or additional speed claims are included in this report.

## Bounded foreground QoS trial

`benchmark-results-qos-trial.json` (`benchmark-results-qos-trial.json`, removed) records two alternating startup pairs comparing the unchanged production release CLI with a temporary copy whose worker thread requests `pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0)` before traversal. Both builds use the same thin-LTO/codegen-units=1 profile and matching libc 0.2.190 / serde_json 1.0.151 dependencies. The temporary binary, source and lockfile are fingerprinted. This is exploratory, not formal acceptance; the API return/applied QoS class was not exposed.

Production ran **19.678 / 19.498 seconds**, versus temporary QoS **19.473 / 19.319 seconds**. Two-run medians were **19.588 versus 19.396 seconds**, a **0.192-second (0.98%) apparent improvement**. Both alternating pairs improved slightly, but the difference is comparable to observed run variation and well short of the roughly one-second gap to OpenDisk. The first pair had entry/issue drift; the second matched entries/issues but allocated bytes differed. This trial provides insufficient evidence to adopt the scheduling change as a performance fix. Production remains unchanged and acceptance remains unmet.

## Further scan tuning: rejected candidates

The following trials preserve requested metadata and traversal policy. Baseline executables and source manifests were saved before editing; candidate executables are hashed before and after each measurement. Initial runs are recorded separately from three alternating warm pairs. All are release-engine trials with eight effective workers unless stated otherwise; OS caches and macOS permissions were not changed by tooling.

| Candidate | Home baseline → candidate median | Startup baseline → candidate median | Decision |
| --- | ---: | ---: | --- |
| Worker-local reusable 256 KiB buffer and borrowed record names | 11.446 → 11.480 s | 18.984 → 19.015 s | Reverted; no useful gain |
| 64 KiB buffer, final production-shaped candidate | 11.540 → 11.458 s | 18.940 → 19.101 s | Reverted; inconsistent gain and startup regression |

Raw reports: buffer reuse (`benchmark-results-buffer-reuse.json`, removed), 64 KiB final trial (`benchmark-results-buffer64-final.json`, removed). An earlier 64 KiB project-scope trial (`benchmark-results-buffer64-projects.json`, removed) suggested a 4.5% median lead, which did not translate to the full scopes. That exploratory isolated build also had a 16-worker cap but ran eight effective workers; the final candidate restored the original eight-worker cap before measurement.

A separate three-round rotated project-scope worker sweep (`benchmark-results-more-workers.json`, removed) tested an isolated scanner copy with a maximum of 16 workers on this 14-core host. Medians were **2.219 s at 8**, **2.273 s at 12**, and **3.084 s at 16**. More workers were rejected. After these trials, the default stayed **8 workers with the original 256 KiB per-directory buffer**. The later worker Settings feature allows an explicit user override.

The scans were incomplete under inherited permissions, and live metadata—including recording artifacts within some scanned scopes—changed between runs. Strict aggregate/path coverage gates remain unverified. These trials establish neither an OpenDisk.app comparison nor the original full-scan acceptance target. A new regression compares every native path, identity, size, and hard-link flag with portable traversal across a large multi-batch directory, unusual names, a symlink, and empty directories.

To repeat a before/after engine trial with captured release executables and their source manifests:

```sh
python3 scripts/speed_trial.py \
  --baseline-bin /absolute/path/to/baseline \
  --candidate-bin /absolute/path/to/candidate \
  --baseline-source-manifest /absolute/path/to/baseline-source.json \
  --candidate-source-manifest /absolute/path/to/candidate-source.json \
  --output /absolute/path/to/report.json
```

`--baseline-workers` and `--candidate-workers` can override the shared `--workers` setting. `scripts/worker_sweep.py --counts 4 8 12 16` supports an explicitly supplied experimental executable and `--source-manifest`; the historical baseline capped concurrency at eight. With worker Settings, the scanner and CLI now share `MAX_WORKERS` (16), while the desktop further caps selections to available CPU threads. The CLI reports its effective count, so tooling rejects an unsupported requested count instead of producing misleading comparisons.

## Map pointer interaction: adopted optimization

The previous Canvas event handler called `hit()` inside `nodes.find(...)`. Each candidate node repeated the shape search and copied/reversed the entire shapes array. A missed pointer on a 5,000-entry map could therefore perform 5,000 shape searches and array copies for one event.

The event handler now searches paint order once, iterating backward without a copy, then looks up the node in a memoized ID map. Selection highlighting also uses a memoized set. Hover, click, and double-click share this lookup; descendant priority, angle/ring boundaries, and non-actionable aggregate behavior remain unchanged.

Raw pointer benchmark (`benchmark-results-map-pointer.json`, removed) uses the actual new helper and an extracted copy of the old lookup with 5,000 synthetic nodes. It retains initial samples and three alternating warm samples, verifies all seven selected IDs, and records script/helper/component hashes and map setup cost separately.

| Pointer case | Previous warm median per event | New warm median per event |
| --- | ---: | ---: |
| Treemap miss | 38.687 ms | 0.00474 ms |
| Sunburst miss | 52.283 ms | 0.00500 ms |
| Treemap first entry | 0.0525 ms | 0.00514 ms |
| Sunburst first entry | 0.0301 ms | 0.00579 ms |

These are synthetic lookup timings in Node 24, excluding DOM reads, React, Canvas drawing, layout workers, IPC, and native scanning. Different batch repeat counts and JIT effects limit precise speedup ratios. They do not represent end-to-end pointer latency or a scan-speed improvement. Browser checks on the real Canvas component confirmed selection in both dense maps; regression tests require exactly one canvas measurement per pointer event, verify refreshed node data, and preserve folder and aggregate actions.

```sh
node scripts/map_pointer_benchmark.mjs --output benchmark-results/map-pointer.json
```

## Worker-scaling diagnostics

The user reported startup-disk times at worker counts 1–14: **67.1, 35.8, 25.1, 20.1, 17.4, 17.2, 17.1, 18.6, 19.7, 22.5, 21.8, 22.8, 24.3, 24.5 seconds**. These are user-reported app observations, not independently repeated medians. A hardware query confirmed **10 Performance and 4 Efficiency** logical/physical cores on this host.

Added per-worker instrumentation to distinguish processing, CPU execution, metadata work and synchronization waits. Percentage contribution means accepted entries divided by total entries minus the root initialized outside workers. Requested QoS is a start-time sample, not actual/effective priority or P/E placement. Apple describes dynamic core scheduling, QoS hints and CPU topology queries in its [CPU scheduling guidance](https://developer.apple.com/videos/play/tech-talks/110147/). CPU time is measured with the calling thread's [CPU clock](https://github.com/apple-oss-distributions/Libc/blob/main/gen/clock_gettime.3), including user/kernel execution but excluding time merely waiting.

Initial diagnostic samples (`benchmark-results-worker-diagnostics-initial.json`, removed) ran fresh release CLI full scans at 5, 7 and 14 workers, with unchanged permissions and no scheduling policy changes. These are one-sample exploratory measurements per count, not app benchmarks or acceptance runs.

| Workers | Elapsed | Average CPU-thread equivalents used by workers | Metadata fraction of summed worker processing wall time | Store-wait fraction |
| --- | ---: | ---: | ---: | ---: |
| 5 | 19.957 s | 2.84 | 97.31% | 0.79% |
| 7 | 19.040 s | 4.57 | 97.39% | 0.98% |
| 14 | 25.813 s | 9.79 | 98.00% | 1.14% |

Metadata includes no-follow directory opening and identity validation, native enumeration, record parsing and path creation. This evidence locates most time in that stage; it does not separate kernel/filesystem contention, allocation/cache behavior, and P/E scheduling. It does not establish the Rust tree write lock as the main bottleneck in these CLI samples. The GUI additionally performs tree/map queries and rendering, whose costs are not captured by worker CPU totals.

A follow-up with descriptor counters (`benchmark-results-worker-fd-diagnostics.json`, removed) checked whether additional workers exhausted the retained-parent budget and caused repeated secure absolute-path walks:

| Workers | Elapsed | Metadata µs per attempted folder | Peak retained handles / limit | Absolute path reopen attempts |
| --- | ---: | ---: | ---: | ---: |
| 7 | 19.024 s | 172.8 | 89 / 512 | 0 |
| 14 | 25.850 s | 472.0 | 169 / 512 | 0 |

Entry/issue accounting and worker IDs matched each scan's totals. Both scans attempted 740,121 folders, including reuse of the held root, with 740,120 relative opens and no absolute fallback attempts. Descriptor pressure therefore does not explain these samples. All worker QoS requests sampled as `default`.

The 14-worker sample had four lower-contribution workers (5, 10, 11 and 13) at roughly 4.9–5.2% of accepted entries; most others were near 7.6–8.2%. That is compatible with heterogeneous execution rates but **does not identify those workers as Efficiency-core workers**. Directory workload composition, scheduling, cache effects and thermal conditions remain confounders. Actual core placement requires an Instruments System Load/Thread State trace; no affinity hacks or priority changes were adopted.

Two alternating overhead pairs (`benchmark-results-diagnostics-overhead.json`, removed), at seven workers on startup, compared the captured pre-instrumentation release executable against the instrumented executable. Baseline times were **18.894 / 19.058 s**, diagnostic times **18.985 / 19.027 s**; two-run medians were **18.976 versus 19.006 s**, a **30 ms (0.16%) difference** within observed variation. Entry/issue counts drifted, and the baseline source manifest was not captured before edits. Its executable hash and an empty-folder probe confirming absent diagnostic fields were preserved. This is an exploratory overhead check, not a claim of exact cost, equivalent coverage, GUI overhead or original OpenDisk acceptance.

```sh
python3 scripts/worker_diagnostics_trial.py --path / --counts 5 7 14 \
  --rounds 3 --output benchmark-results/worker-diagnostics.json
```

The first rotation is retained as initial, later rotations as warm samples; all runs use fresh processes/full traversal. Per-worker raw records, executable hashes, root-excluded accounting, CPU averages and explicitly labeled wall-phase fractions are saved. Increasing workers remains an explicit setting; defaults, QoS, traversal and descriptor policy were preserved during this investigation.


## User-selectable QoS comparison

Settings now offers Automatic, Prioritize scan (`QOS_CLASS_USER_INITIATED`), and Background (`QOS_CLASS_BACKGROUND`) on macOS. Automatic makes no setter call and remains the default. Policies apply once to each newly spawned scan worker, before telemetry starts; no main/UI/coordinator thread priority, affinity, global power policy or filesystem coverage rule changes. Return codes and requested-class samples are validated. QoS does not identify actual P/E-core placement.

`scripts/qos_trial.py` runs the same release CLI executable with rotated policies, one initial rotation and three warm rotations (`--rounds 4`). Every scan is a fresh full traversal of `/`, with inherited permissions and no cache flushing. The 7-worker trial includes all three modes. After Background proved substantially slower, the 14-worker trial compares only Automatic and Prioritize scan. Compilers, development server, and concurrent scans were stopped before timing; the user's completed native app session was preserved.

```sh
python3 scripts/qos_trial.py --path / --workers 7 --rounds 4 \
  --output benchmark-results/qos-7.json
python3 scripts/qos_trial.py --path / --workers 14 --rounds 4 \
  --modes automatic userInitiated --output benchmark-results/qos-14.json
```

These are exploratory instrumented engine measurements, not app-level or OpenDisk acceptance results. Initial runs are separate from warm medians. Matching aggregate totals are checked, but do not prove identical path sets. Live filesystem changes and incomplete coverage limit exact performance ratios; worker classes are requested QoS, not a trace of core placement. Hardware remains 10 Performance + 4 Efficiency cores.


Raw reports: 7 workers (`benchmark-results-qos-settings-7.json`, removed), 14 workers (`benchmark-results-qos-settings-14.json`, removed). All 20 scans completed, exact worker/entry/error accounting passed, the binary hash stayed unchanged, explicit QoS requests returned success and sampled as the requested class. Automatic workers sampled as `default`. The 7-worker warm medians were **17.553 s Automatic**, **17.568 s Prioritize scan**, and **79.849 s Background**. The 14-worker warm medians were **25.718 s Automatic** and **26.218 s Prioritize scan**.

| Workers | Policy | Initial scan | Three warm scans | Warm median | Median peak RSS |
| --- | --- | --- | --- | --- | --- |
| 7 | Automatic | 17.791 s | 17.553 s / 17.745 s / 17.361 s | **17.553 s** | 1.68 GiB |
| 7 | Prioritize scan | 17.624 s | 17.568 s / 17.296 s / 17.808 s | **17.568 s** | 1.68 GiB |
| 7 | Background | 72.309 s | 68.355 s / 79.849 s / 92.766 s | **79.849 s** | 1.41 GiB |
| 14 | Automatic | 25.703 s | 25.718 s / 25.397 s / 27.471 s | **25.718 s** | 1.68 GiB |
| 14 | Prioritize scan | 26.002 s | 25.449 s / 26.218 s / 26.487 s | **26.218 s** | 1.69 GiB |

Prioritize scan was **0.09% slower** than Automatic at seven workers and **1.94% slower** at fourteen. This provides no evidence of a speed benefit and does not fix the higher-worker slowdown. Background's seven-worker median was approximately **4.55×** Automatic's; it is an optional lower-priority mode, not a scan-speed optimization. Automatic remains the default; worker count remains an explicit setting with its existing default.

Coverage was not stable across warm scans, so these are not strict equivalent-coverage comparisons. At seven workers, Automatic/Prioritize scan had 4,111,747–4,111,758 entries and 449 issues, while Background had 4,106,533–4,111,598 entries and 469–497 issues. Longer traversal increases opportunities for namespace changes, but the issue paths/causes were not collected in these raw CLI reports; that is a possible explanation rather than a verified cause. Allocated totals also changed. The fourteen-worker report likewise has aggregate drift. Initial runs and all incomplete-result issue counts remain recorded; matching worker accounting is not proof of complete filesystem coverage.

No actual P/E scheduling trace, energy measurement, or end-to-end GUI benchmark was performed. Original OpenDisk scope/coverage/performance acceptance remains outstanding. The timing results justify preserving Automatic, without claiming an exact universal cost or performance ratio for these policies.


## Automatic worker calibration

Auto defaults to `min(4, available CPU threads)` until a matching volume/hardware/QoS profile exists. Old numeric choices retain Manual mode. Calibration is explicit after a completed scan, preserves that result, and samples directory identities with their required ancestors. It uses production traversal under the same held root, at most 4,096 directory jobs and 250,000 nodes. Preparation makes two fresh bounded captures with a cancellable one-second observation interval, prunes jobs with changing membership and their dependent branches, then freezes the remaining sample. Root-listing drift, metadata errors, later membership changes, short trials or insufficient stable measurements retain the old choice.

The measured candidate range is every available count through the scanner's 16-worker cap. One warm-up rotation is excluded; three rotated warm timings per count determine the median. Each trial must last at least 50 ms; candidates with a warm range above 25% of their median are excluded. Among stable candidates, the lowest count within 5% of the fastest median is selected. The shared 90-second deadline includes preparation and is cooperative. This estimates useful concurrency for a bounded sample; it is not proof of the fastest complete scan under different workload, connection, cache, or thermal conditions.

Final release native-command probes: GitHub scope (`benchmark-results-auto-calibration-projects.json`, removed), startup scope (`benchmark-results-auto-calibration-startup.json`, removed). Both use the same unchanged hashed release test executable and temporary preferences. They exercise start/status/persistence/resolved-worker selection through native commands; no compilers, development server or concurrent scanner ran during timing. Inherited permissions were unchanged, and the open application session/user preferences were preserved. The test harness models Full Disk Access acknowledgement in its own disposable setup; it grants no macOS access. These are not graphical `.app` or OpenDisk acceptance comparisons.

| Scope | Source full scan | Bounded calibration | Next full scan |
| --- | --- | --- | --- |
| GitHub | 4 workers, 3.194 s, 698,640 entries, 0 issues | Selected 5 workers; 4,096 directories / 55,384 entries | 5 workers, 2.347 s, 698,640 entries, 0 issues |
| Startup | 4 workers, 20.247 s, 4,121,816 entries, 449 issues | Declined changed membership after 43/56 trials; retained 4 workers | Not run after declined calibration |

GitHub completed all 56 trials, saved **five workers**, survived a preferences reload, and used five on the subsequent full scan. Source and follow-up entry/issue totals matched. Source-versus-next elapsed times are single ordered observations with different cache conditions and do not establish a causal speedup or a full-scan optimum.

The startup sample still changed during measurement, despite passing preparation. It safely retained **four workers**, preserved the completed result and stored no profile. Partial timing rows remain in the report, but are not used to claim a successful calibration or select workers. This is the remaining Auto limitation on active startup folders. The earlier full-scan worker measurements favor 5–7 on this host; a manual count remains available when calibration cannot obtain a stable sample.

The stable GitHub sample's three-warm medians:

| Workers | Median sample time |
| --- | --- |
| 1 | 185 ms |
| 2 | 97 ms |
| 3 | 84 ms |
| 4 | 69 ms |
| 5 | 57 ms |
| 6 | 63 ms |
| 7 | 65 ms |
| 8 | 67 ms |
| 9 | 70 ms |
| 10 | 79 ms |
| 11 | 80 ms |
| 12 | 85 ms |
| 13 | 91 ms |
| 14 | 97 ms |

Initial source-staleness and back-to-back preparation reports are retained with `-initial` and `-refresh` suffixes. They show safe failure/persistence behavior and motivated the final preparation interval; their timings are not combined with the final candidate.

The probe script that reproduced these runs was retired with the calibration feature; the reports above are historical.

## Calibration error recovery and EINTR (11 October 2026)

The screenshot's generic failure did not retain an underlying cause. The first detailed-error candidate reproduced a startup failure at 13 workers with `EINTR` (OS error 4) in `Library/Containers/com.apple.FinalCut/Data`; see startup reproducer (`benchmark-results-calibration-recovery-startup.json`, removed). Its incomplete timings were discarded, no profile was saved, and the previous four-worker choice remained. The companion project probe (`benchmark-results-calibration-recovery-projects.json`, removed) completed. These reports precede interrupted-call handling and are retained as diagnostic evidence, not mixed into final scores.

The corrected scanner retries only interrupted metadata operations, with at most four attempts. Interrupted directory listings discard successful partial data and restart with an independent identity-validated cursor on the held directory. Existing permanent entry errors remain visible and prevent a retry from concealing an incomplete pass. Calibration separately supports at most three whole attempts for recoverable sample changes or confirmed descriptor pressure, using fresh stabilization and scores each time under the shared 90-second cooperative limit. Independent affected-scope reviews R7 and R8 approved fatal-error precedence and interrupted-listing preservation.

Final probes use the corrected, unchanged release test executable, one scope at a time after builds finish. They preserve user preferences, grants, the open app and its completed results. A successful probe verifies profile persistence/reload and that the following full scan uses the saved count. These are native-command integration observations, without alternating OpenDisk runs or proof that the sampled count is the full-scan optimum. Original performance acceptance remains outstanding.


The final startup probe (`benchmark-results-calibration-eintr-startup.json`, removed) and GitHub probe (`benchmark-results-calibration-eintr-projects.json`, removed) both completed all **56 trials**, selected **five workers**, persisted/reloaded the profile and used five on the next full scan. Both used unchanged SHA-256 `61d42533217f7bd5927d44497a61579ad14bd91748f96b2d73a2b89944d1c261`, with no compiler, dev server or concurrent scanner during measurements.

| Scope | Source full scan (4 workers) | Fixed calibration sample | Next full scan (5 workers) |
| --- | --- | --- | --- |
| Startup | 19.883 s; 4,129,555 entries; 449 issues | 4,096 directories; 139,521 entries; attempt 1; no trial failures | 17. 339 s; 4,129,556 entries; 449 issues |
| GitHub | 3.181 s; 705,299 entries; zero issues | 4,096 directories; 55,002 entries; attempt 1; no trial failures | 2.389 s; 705,299 entries; zero issues |

Startup's sample warm medians were 122 ms at five, six and seven workers, 181 ms at thirteen and 191 ms at fourteen; the fewer-workers rule chose five. GitHub's five-worker warm median was 54 ms. No whole-attempt recovery was necessary in these successful observations; injected tests cover that path. Startup scans still report inaccessible entries, and one entry changed between full scans. The ordered source/next times can reflect cache and workload changes and do not establish a speed improvement, equivalent OpenDisk coverage or full-scan optimality.

## Hardware-based Auto policy (11 October 2026)

The current app supersedes the calibration Auto sections above. Auto uses [OpenDisk's scope-specific worker formulas](https://github.com/137137137/OpenDisk/blob/f7551cf21a5c35a963c607401ba5705ec0869b64/OpenDisk/Services/Scanning/TraversalScanner.swift#L6-L12) plus [Radix's power/thermal reductions](https://github.com/colinvkim/Radix/blob/df1235bfb05acb7a678718166491bcc27817d95d/Radix/Services/ScanEngine.swift#L842-L898). These engineering ideas are independently implemented; no upstream scanner code was copied. The application uses one bounded directory pool and introduces no nested classification pool.

Under normal conditions, a 14-thread Mac gets seven workers for volume roots and six for folders (folders get three to five on smaller CPUs or under unknown conditions, and fewer, down to one, under Low Power Mode or thermal pressure). The rules use active CPU count, not P/E counts. They are deterministic heuristics rather than measured per-volume optima. Low Power Mode and temperature can reduce the next scan's count; settings and actual scan start each read current conditions, so the displayed prediction can change before launch. Active scans retain their captured count.

To check the current policy through native commands, build the release test executable, then use Cargo's printed executable path:

```sh
cargo test --release -p hahakigami --lib --no-run
python3 scripts/workers_probe.py --binary target/release/deps/hahakigami_lib-TEST_HASH \
  --path /absolute/folder-or-volume --output benchmark-results/hardware-auto.json
```

The probe uses disposable preferences and inherited permissions. It runs one full Auto scan, verifies captured worker/source information, and checks that production Auto ignores retired preferences. It hashes the executable before and after, records source fingerprints, and does not change grants or user settings. This is an integration observation, not an alternating full-scan benchmark or a graphical app launch check. Earlier calibration reports remain historical evidence.

Final startup policy probe (`benchmark-results-hardware-auto-startup.json`, removed) and GitHub policy probe (`benchmark-results-hardware-auto-projects.json`, removed) passed with unchanged executable SHA-256 `9114ff4ec09e25152c925f57fdbfdeff0f66f20b466446031d893be8260328eb`. Both reported 14 CPU threads, Low Power Mode off and nominal temperature. Each verified a completed hardware Auto scan, fixed captured workers, and a disposable Manual save/reload.

| Scope | Auto workers | Elapsed | Entries | Issues |
| --- | ---: | ---: | ---: | ---: |
| Startup volume | 7 | 17.761 s | 4,131,159 | 449 |
| GitHub folder | 3 | 4.098 s | 706,871 | 0 |

These are single ordered observations with incomplete startup coverage, not warm-run medians or proof of a performance improvement. Build/compiler and development-server work had finished before either probe. Full `.app` bundling passed; the user's existing process was preserved rather than relaunched. No probe/server remains active, and original OpenDisk performance acceptance remains outstanding.

## Automatic scans without app Settings (11 October 2026)

The Settings interface described in the preceding adoption section is now retired. Production scans always use hardware Auto and Automatic scheduling; older Manual/QoS/calibration records do not influence them. The two settings command grants are removed from the frontend capability. The existing formulas and held-root classification remain unchanged, so their earlier policy tests and startup observations are reused rather than repeated.

The final GitHub release probe (`benchmark-results-settings-removed-projects.json`, removed) passed with unchanged executable SHA-256 `fd20b6d0effe9335d3b5c023a5ae3078fc9bc88183625c7d2bb949abfc0d0f3f`. It completed in **3.958 s**, using **three Auto workers**, Automatic scheduling, **707, 905 entries** and zero issues. Reloading a disposable legacy Manual preference still produced hardware Auto with three workers and Automatic scheduling through the production policy query. Existing unit/native checks additionally cover retired Background/UserInitiated preferences and prove the old file is not rewritten.

This was one native-command integration observation after all compiler/server work finished, not a median or speed comparison. The host release `.app` build passed. Browser-only App verification showed direct explorer startup, retained diagnostics and the bold map size label without Settings navigation. The user's live process, real preferences, in-memory results and macOS grants were preserved. No probe or dev server remains active; the original OpenDisk performance acceptance is still outstanding.

## Isolated performance and memory suite (12 October 2026)

The new suite separates the native scan/cache pipeline from packaged UI measurements. It freezes the original source and scanner executables before optimization; baseline and candidate receive the same benchmark-only instrumentation. Radix is pinned to `df1235bfb05acb7a678718166491bcc27817d95d`, and OpenDisk remains pinned to `f7551cf21a5c35a963c607401ba5705ec0869b64`. Upstream code and licenses stay in the ignored benchmark cache, with retained source patches and executable hashes for isolated UI builds.

`performance-benchmark` is an explicit Cargo feature, absent from ordinary desktop builds. Its standalone `native_performance` example accepts `--config` with schema version 2, absolute `root`/`state_dir`, `scope`, `mode`, and optional absolute `coverage_output`. It calls the normal target registration, hardware Auto, scan, journal/cache and completion pipeline. `full` retains production cache work with a fresh cache; `same_process` prepares a prior result in the same process; `persisted` is prepared by a separate process before the measured process starts; `changed` additionally validates and mutates an owned fixture. Actual reuse and full-fallback reasons remain visible. Preparation is outside the timed scan; same-process preparation remains in that process's RSS high-water.

Build an isolated Hahakigami UI variant through the signing helper:

```sh
pnpm desktop:build --benchmark-output /absolute/new/benchmark-build
```

The helper retains the normal bundle identifier and stable Apple Development designated requirement, places all build outputs in the owned benchmark directory, and configures an incognito webview. The instrumented app requires `--config`; it uses disposable preferences, library and cache locations. Its setup acknowledgement models saved setup without asserting an OS grant. Normal builds retain the original setup workflow and expose no benchmark command or environment override. Neither build launches the user's existing application.

`build_radix_benchmark.py` and `build_opendisk_benchmark.py` compile adapters against pinned upstream scanner sources. `build_upstream_apps.py` builds development-signed UI variants with private persistence and updates/privacy probes disabled at bootstrap. The retained patches document all differences. Radix UI runs support full scans; OpenDisk supports full/seed/persisted. Unsupported modes fail before scanning. Full measurements use fresh state on every round; rescan modes use separate owned mode directories. Upstream counts, worker limits and native-path limitations are reported rather than assumed equivalent to Hahakigami.

`performance_fixture.py --parent /absolute/disposable-parent` creates 100,000 entries excluding the root on a verified APFS volume, with shallow/deep/wide directories, sparse files, hard links, symlink cycles, Unicode/newline names and a clone. The host rejects invalid UTF-8 names; that limitation is retained, and byte fidelity is tested independently. Mutation checks creation and seeded identities through held no-follow descriptors; substituted files, hard links and ancestors cannot authorize outside writes.

Run an explicit executable/scope configuration:

```sh
python3 scripts/performance_suite.py --config /absolute/suite.json --output /absolute/report.json
```

Executables declare `id`, `role` (`baseline`, `candidate`, `reference`), `layer` (`native`, `app`), absolute `path`, optional arguments/source paths and supported modes. The config lists absolute scope paths, requested modes, five warm rounds and bounded timeout. Optional `measurement_modes` selects a retry subset while acceptance still evaluates every requested mode. Optional `coverage_scopes` limits untimed exports; omitted scopes remain inconclusive. Atomic checkpoints retain completed rows with `complete: false` and `acceptance_met: false` until finalization. One initial round is separate; five warm orders rotate the executables. Processes run sequentially, without builds, profiling, cache flushing, affinity changes or privacy modifications. Persisted runs launch an owned seed process and then a fresh measured process.

Native peak RSS comes from `getrusage`; packaged runs additionally sample the owned PID and verified descendant births every 100 ms. Aggregate RSS can double-count shared pages and omits reparented/shared helpers, so it is not physical footprint. Tauri completion waits for final list/map data and current Canvas layout/drawing. The corrected cohort emits at that CPU drawing boundary; it avoids animation-frame waits that can pause when an owned window is occluded. Same-process/changed packaged preparation uses a real completed UI draw before the measured rescan, with one-shot event coordination and stale-reply guards. Upstream callbacks observe the actual final chart Canvas with layout/content checks. These are CPU draw barriers, not GPU presentation measurements. Launch clocks have explicit origins; scan timing excludes launch. OpenDisk exposes post-draw background-cache publication wait separately, with memory observed through publication.

Coverage export is a separate untimed pass, streamed as raw-path-base64 NDJSON with declared node count, directory root and completed footer. The SQLite comparator bounds its memory and reports policy/size-scope differences, missing paths, mismatch samples, summary boundaries and unknown fields. Stable aggregates alone never establish equivalence. Recorded total and retained issue counts, bounded native-path/error samples and excluded branch samples are separate diagnostics. Incomplete exports remain inconclusive; redundant self-equivalence checks are skipped with an explicit unverified-stability status. Upstream Foundation/Swift path fidelity, Radix summaries and clone accounting can leave external equivalence inconclusive.

Acceptance requires complete stable baseline/candidate coverage, unchanged hashes, finite positive metrics, correct requested/actual modes, five successful warm rounds, at least 20% lower native and packaged peak memory on Home and startup, and no more than 5% timing regression, including rescan scenarios. Failed, truncated, malformed, missing or incomparable evidence is inconclusive. Raw measurements remain available even when gates cannot pass. Exact-path candidates declare an executable storage contract checked on full runs, catching stale linked scanner artifacts. Release scanner packages must be rebuilt after copying build caches and preflighted before collecting timings. Final candidate-matched outcomes are recorded below after measurements complete. Intervention flags invalidate affected acceptance rows. The retained incident ledger records the first runner’s rescan state-ownership failures and one occlusion-delayed packaged completion; successful full rows are retained separately from retries.


### Pinned reference observations

These are five warm medians from the first cohort, with normal scanner defaults and isolated UI persistence. Native memory is process peak RSS; app memory is sampled owned-process/verified-descendant aggregate RSS. The Hahakigami candidate in that cohort was invalidated by stale linked scanner artifacts. Reference rows remain raw observations, without an equivalent-coverage speed or memory ranking. External power/thermal readings were not recorded at process boundaries in that cohort.

| Scope | Reference/layer | Scan time | RSS measure | Coverage |
| --- | --- | ---: | ---: | --- |
| fixture | radix_native | 0.426 s | 0.146 GB | incomparable |
| fixture | opendisk_native | 0.046 s | 0.025 GB | incomparable |
| fixture | radix_app | 0.773 s | 0.265 GB | incomparable |
| fixture | opendisk_app | 0.094 s | 0.100 GB | incomparable |
| home | radix_native | 26.919 s | 3.966 GB | incomparable |
| home | opendisk_native | 18.898 s | 0.894 GB | incomparable |
| home | radix_app | 25.745 s | 4.420 GB | incomparable |
| home | opendisk_app | 19.394 s | 1.460 GB | incomparable |
| startup | radix_native | 38.484 s | 4.508 GB | incomparable |
| startup | opendisk_native | 17.702 s | 0.924 GB | incomparable |
| startup | radix_app | 36.482 s | 4.826 GB | incomparable |
| startup | opendisk_app | 18.599 s | 1.668 GB | incomparable |

Radix summarization, native-path representation, directory size scope and clone accounting differ. OpenDisk does not expose original native bytes, symlink kinds in this scanner interface; configured worker bounds are reported, not observed active worker counts. Missing paths and accounting mismatches remain in the coverage report. Shared/reparented helpers are unattributed.


## Corrected performance and memory results

Full report (`benchmark-results/performance/suite-corrected-report.json`, local only), compact summary (`benchmark-results/performance/suite-corrected-summary.json`, local only), configuration (`benchmark-results/performance/retry-config.json`, local only) and measurement incident ledger (`benchmark-results/performance/measurement-incidents.json`, local only) identify this candidate. The report is complete with `acceptance_met: false`: fixture gates pass, while Home/startup gates are inconclusive because complete stable coverage was not verified. Independently, startup's raw full-scan memory reductions miss 20%. Optimization stops at three investigated candidates.

The corrected cohort ran after clean scanner rebuilds and successful native/UI path-capacity preflights. All 240 timed runs succeeded: 96 fixture, 72 Home and 72 startup, with zero interventions or filtered samples. All 136 source/binary fingerprints matched before and after. Each scenario has one initial observation and five rotating warm rounds; the tables use warm medians only. Every recorded process-boundary thermal state was nominal and Low Power Mode was off. Hardware Auto used three workers on folders and seven on startup, with Automatic scheduling. The host is the 14-thread Mac recorded above.

### Full scans

Native memory is process peak RSS; app memory is sampled owned-process/attributable-descendant aggregate RSS. GB means 1,000,000,000 bytes. These measures are separate, and aggregate RSS is not physical footprint. Reparented or shared helpers remain unattributed. Timing covers the normal native pipeline or scan initiation through completed CPU-drawn UI, excluding launch. Fixture memory has no 20% acceptance gate.

| Scope/layer | Baseline → candidate time | Baseline → candidate RSS (GB) | Memory reduction | Time change |
| --- | ---: | ---: | ---: | ---: |
| fixture/native | 0.153 → 0.157 s | 0.066 → 0.058 | 12.55% | +2.45% |
| fixture/app | 0.164 → 0.166 s | 0.133 → 0.102 | 23.05% | +1.27% |
| home/native | 19.987 → 19.702 s | 1.575 → 1.255 | 20.36% | -1.43% |
| home/app | 20.031 → 20.025 s | 1.661 → 1.266 | 23.77% | -0.03% |
| startup/native | 18.912 → 18.930 s | 2.547 → 2.120 | 16.76% | +0.09% |
| startup/app | 19.119 → 19.025 s | 2.648 → 2.203 | 16.80% | -0.49% |

Home's raw thresholds pass in both layers, while startup's memory reductions fall short by about 3.2 percentage points. No complete-coverage Home/startup claim is made. Initial runs, per-round launch times, accounting totals, worker diagnostics, phase timings and sampled memory peaks remain in the report.

### Rescans and cache limitations

Ratios are candidate/baseline; the limit is 1.05. Changed-folder runs mutate only the owned fixture.

| Scope/requested mode | Native time ratio | Packaged time ratio |
| --- | ---: | ---: |
| fixture/same_process | 0.97331 | 0.99706 |
| fixture/persisted | 1.01311 | 1.00523 |
| fixture/changed | 1.00712 | 1.02247 |
| home/same_process | 0.99873 | 1.03616 |
| home/persisted | 0.99740 | 0.99055 |
| startup/same_process | 0.99196 | 0.97849 |
| startup/persisted | 1.00724 | 1.02773 |

Every observed actual cache mode was `full`. These are measurements of rescan requests and conservative fallbacks, not demonstrated cache-hit acceleration. A separate clean-folder diagnostic (`benchmark-results/performance/cache-diagnostic-report.json`, local only) observed cache publication and journal replay, but event flags `0x410900` required a full scan. Cache decoding/reuse is covered by tests; no successful end-to-end cache reuse was observed in these runtime cohorts.

Packaged same-process timing varied substantially: Home medians were 58.843 → 60.971 seconds, with a retained candidate warm outlier of 87.487 seconds. Startup medians were 30.773 → 30.111 seconds. Recorded scan/enumeration/aggregation time dominated the slow samples; no unverified cause, focus intervention or scheduling override is used to explain them away. All rescan median timing limits pass as raw observations.

### Coverage outcome

Both fixture layers have complete, equivalent, stable repeated coverage, including native names, sparse files, hard links, symlink cycles and the APFS clone. Home/startup exports finished, but access issues, excluded branches and live filesystem differences prevent complete stable coverage. Recorded metadata differences are not filtered to make coverage pass.

| Layer/scope | Issues per side | Excluded per side | Missing baseline-only/candidate-only | Allocated/identity mismatches |
| --- | ---: | ---: | --- | --- |
| native/home | 134 | 0 | 0 / 0 | 0 / 5 |
| app/home | 134 | 0 | 0 / 0 | 1 / 19 |
| native/startup | 440 | 11 | 2 / 4 | 4 / 15 |
| app/startup | 440 | 11 | 2 / 4 | 5 / 18 |

Policies match within each baseline/candidate pair; kind, summary and size-scope mismatches and unknown fields are zero. Bounded recorded permission-error and excluded-branch samples remain in the report. Live log/cache differences and a documentation edit during Home enumeration remain filesystem activity limitations; no scanner source or measured binary changed. Neither setup acknowledgement nor an unchanged signing requirement proves OS access. External observations above remain incomparable, including Radix summaries and OpenDisk's size/kind/native-path limitations.

### Candidate selection and delivery

The candidate ledger (`benchmark-results/performance/candidates/storage/candidate-ledger.json`, local only) retains all three experiments. Path shrinking was superseded after a bare-fixture RSS reduction of 6.27% and time increase of 1.27%. Bounded node-vector growth was removed after only 0.16% lower RSS and 4.53% slower time. These preliminary fixtures used eight workers and are not native-pipeline/app acceptance measurements. Exact child-path construction is retained: it preserves the node representation and traversal authority while reducing Home retained path capacity from about 719 MB to 424 MB, and startup from 984 MB to 578 MB.

The corrected bare-scanner diagnostic (`benchmark-results/performance/candidates/exact-join/fixture-matched.json`, local only) observed 14.63% lower fixture RSS with 0.23% slower time and exact accounting parity. The unchanged million-node synthetic workload (`benchmark-results/performance/candidates/exact-join/synthetic-matched.json`, local only) observed 52 → 53 ms construction, 7 ms aggregation, 3 ms sorted-page queries and identical 205,930,496-byte median RSS. Requested storage capacities exclude allocator overhead, scan scratch and caller indexes; they do not establish a compact-tree result. No fourth candidate or compact representation was attempted.

A separate fresh-process stack profile (`benchmark-results/performance/profile-native-home.json`, local only) is diagnostic only and excluded from acceptance timing. It recorded exact path capacities and substantial metadata-enumeration work. The first cohort's stale linked candidate builds, rejected rescan state preparation and intervention remain preserved rather than mixed into the corrected comparison.

The normal development-signed bundle is `target/release/bundle/macos/Hahakigami.app`. It was built through `pnpm desktop:build` after a clean scanner rebuild, passed strict signature verification and has the same designated requirement as the frozen normal baseline. Delivery receipt (`benchmark-results/performance/delivery-receipt.json`, local only) records the executable and core-source hashes. It was not launched over the existing session. User preferences, in-memory results, signing identity and privacy grants were preserved.

## Incremental replay follow-up (13 October 2026)

The earlier cohort's all-full result remains historical evidence. Replay previously
rejected every clone/hard-link event across the selected volume before checking
scope. Native reproduction also found ancestor directory creation/metadata events
forcing full scans. Replay now uses public FSEvents extended inode IDs to resolve
external hard-link changes against the exact cached tree; missing identities and
unsafe history still force full scans. Clone notifications are path-scoped under
the existing per-file allocation model. Detailed directory creation/metadata events
refresh listings, while rename/removal events invalidate descendants.

An opt-in, read-only native check on the existing `src-tauri/permissions` subtree
observed **33 reused entries and one reread directory** for both same-process
reuse and a new backend instance loading the persisted cache. Both results matched
fresh full scans by exact native path, identity, own and aggregate allocated sizes. Observed debug-build elapsed times were 249 ms and 505 ms; these tiny-scope
observations do not establish a speed improvement or packaged-app performance.
A separate check on `crates` also observed positive reuse in both paths.

The disposable mutation fixture matched fresh results after additions, removal,
rename, growth, outside hard-link writes and outside APFS clone creation/write/
deletion. A separate native event check retained both affected inode IDs when one
external alias path was repeatedly replaced. Fresh fixture creation remains in
FullHistory overlap and can legitimately yield incremental mode with zero reused
entries. No historical events are discarded based on their cursor ID.

Reproduce on macOS, using an existing clean folder with unchanged subdirectories:

```sh
HAH_CACHE_REUSE_ROOT="$PWD/src-tauri/permissions" cargo test -p hahakigami native_existing_folder_cache_reuse -- --ignored --nocapture
cargo test -p hahakigami native_rescan_cache_parity -- --ignored --nocapture
cargo test -p hahakigami native_external_alias_replacement -- --ignored --nocapture
```

The native commands need access to the public FSEvents service; a restricted
execution sandbox may return no event cursor. They write temporary caches/fixtures
and do not change privacy grants or launch the user's application. Startup scans,
incomplete scans, cached hard links and scopes containing the cache retain their
full-scan policies. Whole-Home/startup and packaged-app acceleration remain unproven.

### Signed app and 100,000-entry rescan comparison

The normal app was rebuilt through `pnpm desktop:build`; its development signature
passed strict verification and its designated requirement matches the prior bundle.
A separate signed benchmark app uses disposable scanner state and private WebKit
storage. The normal app was not launched over the existing user session.

Portable results (`benchmark-results-incremental-summary.json`, removed) record 36 successful
timed runs: one initial round and five warm rounds for each mode/layer, rotating
mode order. Source/binary fingerprints (91) remained unchanged. Initial results
are retained separately; the table shows warm medians. The owned APFS folder has
100,001 entries including the root, twelve directories and mostly empty files.
The OS filesystem cache was not purged. Invalid UTF-8 names were rejected by the
filesystem; the fixture used a Unicode substitute and records that limitation.

| Mode | Native pipeline | Packaged app through final CPU draw | App time / full |
| --- | ---: | ---: | ---: |
| Full scan | 185.60 ms | 205.86 ms | 1.00× |
| Same-process incremental | 258.65 ms | 282.13 ms | 1.37× |
| Persisted-cache incremental | 569.36 ms | 626.71 ms | 3.04× |

Every incremental sample used `incremental` mode, reused **99,996 entries** and
reread one directory; full scans reused zero entries and read twelve directories.
Separate repeated exports verified stable, equivalent native paths, identities,
kinds and own allocated sizes for each incremental mode against full scans
in both layers. Reuse is real, but end-to-end rescanning is slower on this folder.

App median scan/aggregation work fell from about 58 ms to 22 ms. Same-process
journal replay costs about 117 ms. Persisted runs add about 215 ms archive decoding
and a second replay of about 116 ms. Cache publication costs about 131–140 ms in
all modes, including full scans. Phase medians are diagnostic and do not sum
exactly to the end-to-end median. Timings exclude launch, preparation scans and
coverage serialization; persisted preparation runs in a separate process. App
completion means the final Canvas CPU draw, not GPU presentation.

This dataset strongly favors bulk enumeration, so these measurements do not
establish speed on directory-heavy trees, Home, startup or other hosts. Memory
results are not compared because same-process peaks include preparation and
helper attribution remains incomplete. The next performance work should address
journal/decode overhead and cache publication costs rather than assuming positive
reuse is sufficient for faster rescans.

Local raw records, twelve coverage exports, isolated state, reproduction driver
and signed benchmark bundle are retained under ignored
`benchmark-results/incremental-rescan-2026-10-13/`. The checked-in portable summary
contains aggregate results and executable hashes without absolute fixture paths
or raw file listings.

### Optimized incremental rescans (13 October 2026)

Portable results (`benchmark-results-incremental-optimized-summary.json`, removed) record the
final optimized candidate against frozen native and signed-app baseline binaries
on the same owned 100,001-entry APFS tree. There were 72 successful timing runs:
one initial observation and five warm rounds per role, mode and layer, rotating
all six role/mode cases. Current candidate-source/helper and executable fingerprints
(94) stayed unchanged; baseline source hashes were frozen separately before edits.
Repeated untimed exports proved stable exact paths, kinds, identities and own sizes
within each role/mode and between baseline and candidate full scans. All checks
passed. No compilers or additional benchmark scans ran during timing.

| Mode | Frozen native | Optimized native | Frozen app | Optimized app |
| --- | ---: | ---: | ---: | ---: |
| Full scan | 188.24 ms | 116.80 ms | 211.69 ms | 131.67 ms |
| Same-process incremental | 261.62 ms | 89.46 ms | 263.88 ms | 100.17 ms |
| Persisted-cache incremental | 593.15 ms | 100.13 ms | 641.35 ms | 109.72 ms |

The optimized same-process/persisted rescans are **23.4% / 14.3% faster than the
optimized full native scan**, and **23.9% / 16.7% faster than the optimized full
app scan**. Every incremental sample used actual incremental mode, reused 99,996
entries and reread one directory. The comparison uses each candidate layer's own
full scan rather than claiming a win against only the older, slower baseline.

The private optimization cache now encodes node records as a bounded, checksummed
binary stream. App persisted
cache decoding fell from about 214 ms to 14 ms in this cohort. Index and cursor
JSON reads are buffered inside their existing byte caps, following the
[Serde JSON buffering guidance](https://docs.rs/serde_json/latest/serde_json/fn.from_reader.html).

One callback-driven historical stream remains alive through decoding, private
candidate reconstruction and the final flush/drain. This avoids a second stream
startup and overlaps CPU work with daemon delivery. The original validated baseline
is retained, and any relevant final path/directory change discards the private
candidate before ordinary reconciliation. Late-write, external-alias and cancellation
checks protect that decision. No overlapping history IDs or unsafe events are ignored.

When the reconstructed metadata is unchanged, the private archive stays in place
and only its cursor/index is atomically refreshed. This requires a bijective fresh
root listing, canonical names, validated reuse of every descendant listing and
matching own/aggregate accounting. The original full-scan age is never extended.
Missing or mismatched publication records cause a normal fresh cache publication.
App incremental publication medians fell from approximately 135–141 ms to 28–30 ms.
The journal/decode/reconstruction spans overlap and their medians are not additive.

The intermediate compact-codec/single-stream candidate improved the baseline but
missed the speed goal against its own full scans. Its report is retained separately
under `benchmark-results/incremental-optimized-2026-10-13/intermediate/`; its timings
are not pooled with this final cohort. Zero-latency and NoDefer trials did not help
and were discarded. Full-volume scope, history completion, queued-event draining,
namespace/ancestor safeguards, identity validation and conservative fallbacks remain.

The normal app was rebuilt through `pnpm desktop:build`, verified with its existing
Apple Development signature and unchanged designated requirement, and was not
launched over the user's session. The final isolated signed app uses private
WebKit storage and disposable state. Native and app preparation/launch/export time
is excluded; app timing ends at final Canvas CPU drawing. Initial observations,
raw rounds, conditions, phase timings, coverage exports and hashes remain under
ignored `benchmark-results/incremental-optimized-2026-10-13/`.

The speed goal is met for this unchanged synthetic folder on this host. This is
not a general claim for changed scopes, other directory shapes, Home/startup or
other macOS versions. Speculation adds transient snapshot memory and cooperative
cancellation delay; memory is not ranked because preparations and helper attribution
have separate limitations. Preferences and privacy grants
remain intact; old disposable optimization caches are rebuilt in the new format.


## Whole Documents/GitHub measurement (2026-10-13)

The existing optimized native and isolated signed app benchmark binaries were
measured on the user's entire `Documents/GitHub` folder, including dependencies
and build artifacts. No branches or hard-linked files were excluded. All state,
logs and coverage exports stayed outside the scan scope under `/private/tmp`.
The driver used `python3 -B` for the measured cohort; its earlier preflight import
may have created helper bytecode before measurement. No source or binary changes
occurred during the cohort, and the user's running app session remained intact.

There were 36 timed observations: one initial and five warm samples for each
layer/mode, with mode order rotated. The following medians exclude initial samples,
launch, preparation scans and coverage export. OS filesystem caches were not purged;
native timing ends at scanner completion and app timing at final Canvas CPU drawing.

| Requested mode | Native median | App median | Actual mode / reused entries |
| --- | ---: | ---: | --- |
| Full | 5.364 s | 5.306 s | Full / 0 |
| Same-process rescan | 5.239 s | 5.381 s | Full / 0 |
| Persisted-cache rescan | 5.316 s | 5.365 s | Full / 0 |

Every timed scan returned 989,498 entries, zero errors and complete coverage.
Each layer's two full exports matched exactly, and each requested rescan export
matched that layer's full reference: no missing paths or differences in kind,
allocated bytes, identity, summary or size scope. Source/helper/binary
fingerprints remained unchanged. These comparisons are within each layer.

**No actual incremental scan occurred.** Every rescan request reported
`No compatible recent clean scan is cached`. Read-only metadata samples confirmed
hard links in dependencies and build artifacts, and the current snapshot eligibility
rejects any hard-linked node. Consequently this cohort measures full-scan fallbacks;
the small timing differences do not demonstrate incremental acceleration. Hard-link
cache eligibility blocks the journal/decoding improvements from benefiting this
whole real-world folder. No cache-policy changes were made for this measurement.

Portable results are in `docs/benchmark-results-github-summary.json`. Raw observations,
coverage exports, hashes and the local driver remain at
`/private/tmp/hahakigami-github-benchmark-610a7dbd/`; raw path samples were not copied
into repository documentation. The earlier synthetic speedup remains a separate result.


## Whole Documents/GitHub with hard-link reuse (2026-10-13)

Complete, clean local-folder caches now support known hard-link groups. Cached paths
retain raw own sizes, and final aggregation reelects the lexicographic allocation
owner. Dirty aliases, ancestor subtrees and direct listings invalidate every cached
member of the affected device/inode. Dirty paths/listings absent from the baseline
refresh all known groups, covering visible new aliases and transient aliases removed
before enumeration. Newly discovered inode groups still retry a full scan through
the same held root. Hash sets and ancestor matching bound invalidation by path depth.

Volume replay matches all available file IDs, including events without a hard-link
flag. Ambiguous external file identities or directory namespace changes refresh all
cached hard-link members through held descriptors and discard private speculation.
Explicit hard-link records without IDs and unsafe/dropped/oversized history retain
conservative full fallbacks. Existing coverage, identity, epoch and age guards remain.

The normal and isolated apps were rebuilt with `pnpm desktop:build`; existing Apple
Development signatures verified, and the normal app retained its designated
requirement. The user's running session was preserved. Benchmark build output,
state, reports and exports stayed outside the scan scope; Python bytecode writes
were disabled. All compilation finished before timing. The native example used
`performance-benchmark,tauri/custom-protocol`, matching the app's protocol feature.

The whole requested folder was scanned without exclusions or synthetic replacement.
Rebuild/test artifacts changed it from the earlier 989,498-entry cohort; comparisons
are therefore within this final cohort. Thirty-six observations comprise one initial
and five warm samples per layer/mode in rotating order. Medians exclude initial
samples, preparation, process launch and export. Filesystem caches were not purged;
app timing ends at final Canvas CPU drawing.

| Requested / actual mode | Native warm median | App warm median |
| --- | ---: | ---: |
| Full / full | 5.454 s | 5.520 s |
| Same-process / incremental | 5.393 s | 5.501 s |
| Persisted / incremental | 5.760 s | 5.795 s |

All 24 rescan requests were incremental with positive reuse and no fallback,
reusing 865,211–868,145 entries (approximately 87%). Every timed or exported scan had
992,608 entries, zero errors/exclusions, complete coverage, 248,295,698,432 allocated bytes. Repeated full exports and rescan exports
matched exactly within each layer in raw paths, kind, own sizes, identity and size
scope; native/app full exports also matched. Exports record own metadata; matching
root totals separately check deduplicated accounting. Regression fixtures compare
per-path accounted sizes and hard-link flags. Source/helper/binary hashes stayed
unchanged throughout the final cohort.

**Reuse works; this dataset does not show a clear same-process speed gain.** Median
differences were approximately 1.1% native and 0.4% app faster, while persisted
rescans were approximately 5.6% native and 5.0% app slower than full scans. Five warm
samples do not establish significance. Conservative alias refresh discarded private
reconstruction: incremental runs paid approximately 1.7 seconds of speculation,
then 3.1–3.3 seconds of ordinary reconciliation. Persisted decoding/validation cost
approximately 230–235 ms and publication 344–349 ms. Replay/decode/speculation spans
overlap, so their medians are not additive. Avoiding redundant reconstruction when
alias refresh is required is the next measured performance opportunity.

A pre-correction cohort was interrupted after review found a public path-only
new-alias stale-size case. Its observations/binaries remain separate under
`superseded/` and are excluded from final medians. Portable results are in
`docs/benchmark-results-github-hardlinks-summary.json`; raw artifacts remain under
`/private/tmp/hahakigami-github-hardlinks-h612x7gu/`.


## Whole Documents/GitHub after deferring hard-link reconstruction (2026-10-13)

Baselines containing hard links now skip private speculative reconstruction. The
existing live historical stream still runs during cache loading, then flushes and
drains; final alias resolution and epoch validation precede one authorized core
reconciliation against the original baseline. Other scopes retain established
speculative overlap and late-write rejection. Alias invalidation, descriptor
validation, allocation ownership, cancellation and full-fallback guards are unchanged.

The normal app and isolated benchmark app were rebuilt with `pnpm desktop:build`.
Their Apple Development signatures verified; the normal app's designated requirement
remained unchanged, and the user's session was preserved. The native release example
used `performance-benchmark,tauri/custom-protocol`. Build output, benchmark state,
logs and exports stayed outside the requested whole `Documents/GitHub` scope.
Python bytecode writes were disabled; compilation stopped before measurements.

Thirty-six timed observations comprise one initial and five warm samples per
layer/mode in rotating order. Medians exclude initial samples, preparation, launch
and export. Filesystem caches were not purged. App timing ends at final Canvas CPU
drawing. Rebuild/test artifacts changed the entry count slightly from the earlier
cohort; speed comparisons below use this candidate's own full scans.

| Requested / actual mode | Native warm median | App warm median | App reduction vs full |
| --- | ---: | ---: | ---: |
| Full / full | 5.514 s | 5.522 s | — |
| Same-process / incremental | 3.901 s | 3.611 s | 34.6% |
| Persisted / incremental | 3.878 s | 3.988 s | 27.8% |

Native reductions were 29.3% same-process and 29.7% persisted. All 24 rescan
requests were actually incremental, reused 865,484–868,164 entries (approximately
87%), and had no fallback or speculative-reconstruction span. The runner rejects
that span if present, including on untimed coverage runs. Every timed or exported scan
had 992,883 entries, zero errors/exclusions, complete coverage, 248,362,164,224 allocated bytes. Repeated full and rescan exports
matched exactly within each layer; native/app full exports also matched. These
exports compare raw paths, kind, own sizes and identity; matching deduplicated root
totals and regression tests separately protect accounted sizes/hard-link flags.
Source/helper/binary fingerprints stayed unchanged throughout the final cohort.

The prior hard-link cohort paid roughly 1.7 seconds for discarded speculation.
That phase is absent here. App cache-check medians are now approximately 237 ms
same-process and 396 ms persisted, with 225 ms persisted decoding/validation.
Ordinary reconciliation is approximately 2.99–3.22 seconds and publication
342–347 ms. Replay and persisted decoding overlap, so phase medians are not additive.
The previous cohort is retained separately; its different dataset is not pooled or
used as a controlled before/after speed claim. Five warm samples on this unchanged
folder do not imply universal speedups for active changes, other scopes or hosts.

Portable results are in `docs/benchmark-results-github-alias-deferral-summary.json`.
Raw results, state, hashes and eight coverage exports remain under
`/private/tmp/hahakigami-github-alias-deferral-q2ipogy3/`.


## Whole Documents/GitHub after dense seed preparation (2026-10-13)

Prior traces showed approximately 1.5–1.6 seconds of incremental reconciliation
outside worker traversal and aggregation. Public stack sampling of the task-owned
baseline scanner confirmed `Reuse::new` and path hashing as hot spots; this diagnostic
run is excluded from all timing medians. The new constructor propagates invalidation
through validated parent-before-child IDs, then propagates dirty ancestor listings
in reverse. Listing hashes apply only to directories, and cached children stage by
parent ID before the final path/identity lookup map is built. Every reused directory
still opens through held-descriptor identity validation.

Unknown/transient alias refresh, all-member inode invalidation, root freshness,
allocation ownership, cancellation/private publication and new-group full retry
remain. An independent eligibility oracle compares the old path-based rules across
mixed cases, including root-wide changes, direct listings and file listing requests.
Max review found that duplicate directory paths could collide in staging and omit
children. `valid_seed` now rejects all duplicate paths before the fast constructor;
public duplicate-directory/disjoint-child and duplicate-file regressions require
full fallback and exact fresh-scan parity.

Normal and isolated apps were rebuilt with `pnpm desktop:build`. Apple Development
signatures verified, the normal designated requirement remained unchanged, and the
user's running session was preserved. The native example used
`performance-benchmark,tauri/custom-protocol`. Benchmark builds, state, logs and
exports stayed outside the whole requested `Documents/GitHub` folder. Python
bytecode writes were disabled and compilation stopped before timing.

Thirty-six observations comprise one initial and five warm samples per layer/mode,
in rotating order. Medians exclude initial samples, preparation, launch and export.
Filesystem caches were not purged; app timing ends at final Canvas CPU drawing.
Rebuild artifacts changed the dataset slightly; speed comparisons below use this
candidate's own full scans, without pooling prior cohorts.

| Requested / actual mode | Native warm median | App warm median |
| --- | ---: | ---: |
| Full / full | 5.516 s | 5.601 s |
| Same-process / incremental | 3.025 s | 2.949 s |
| Persisted / incremental | 3.045 s | 2.740 s |

App reductions against full were approximately 47.4% same-process and 51.1%
persisted; native reductions were approximately 45.2% and 44.8%. The relative
persisted/memory ordering varies across five samples and does not mean decoding
has no cost. All 24 rescan requests were incremental with positive reuse, no
fallback and no speculative phase, reusing 868,165–868,182 entries (approximately
87%). Every timed or exported scan had 996,424 entries, zero errors/exclusions and
complete coverage. Deduplicated allocated totals matched across all
runs. Eight exports gave exact own-path/kind/size/identity parity within each layer
and between native/app full references. Regression fixtures separately compare
per-path accounted sizes and hard-link flags. Source/helper/binary hashes remained
unchanged throughout the final cohort.

The serial residual is approximately 0.61–0.63 seconds, versus 1.5–1.6 seconds in
the prior traces. It is the per-run reconciliation span minus the maximum worker
elapsed time and final aggregation, then the median: it includes validation and
publication plumbing rather than isolating `Reuse::new`. The prior dataset differs,
so this is diagnostic evidence, not a pooled controlled before/after comparison.
Full and incremental comparisons remain candidate-matched. The speed results apply
to this unchanged folder/host; active namespace changes retain conservative retries.

Portable results are in `docs/benchmark-results-github-reconciliation-summary.json`.
Raw observations, profile, state, hashes and exports remain under
`/private/tmp/hahakigami-github-reconciliation-00ga7ugk/`.


## Whole Documents/GitHub after cached-leaf bookkeeping changes (2026-10-13)

Public stack sampling of the task-owned baseline scanner showed `openat` dominating
worker traversal. Fresh child opens remain mandatory. The new queue validates and
encodes each single-component child name once, preserving raw bytes and rejecting
empty/dot/dotdot/slash/NUL names as explicit errors. Only the initial root can borrow
a parent without a child name. Cached listing selection before opening is data-only;
metadata and reuse counters are used only after fresh no-follow open and
fstat device/inode/kind validation succeeds.

Cached directories with no directory children hold their fresh FD locally without
allocating a retained-parent Arc/budget lease. The leaf flag derives from the same
owned entries during staging. Cached branches and all fresh/dirty listings retain
the existing parent leases, pressure fallback and bounds. Every directory keeps
O_RDONLY, O_DIRECTORY, O_NOFOLLOW and O_CLOEXEC checks. Alias invalidation,
private publication, cancellation and full retries remain. Worker counts and global
queue scheduling were unchanged. Snapshot descriptor peaks measure retained leases,
not all transient active descriptors.

Normal and isolated apps were rebuilt with `pnpm desktop:build`; development
signatures verified and the normal designated requirement was unchanged. The user's
session was preserved. Native release features were
`performance-benchmark,tauri/custom-protocol`. Build output, state, logs and exports
stayed outside the entire requested `Documents/GitHub` scope. Python bytecode writes
were disabled; compilation stopped before measurements.

Thirty-six observations comprise one initial plus five warm samples per layer/mode,
in rotating order. Medians exclude initial samples, preparation, launch and export.
Filesystem caches were not purged; app timing ends at final Canvas CPU drawing.
Rebuild artifacts changed the dataset slightly, so comparisons below use this
candidate's own full scans, without pooling earlier cohorts.

| Requested / actual mode | Native warm median | App warm median |
| --- | ---: | ---: |
| Full / full | 5.507 s | 5.586 s |
| Same-process / incremental | 3.061 s | 3.095 s |
| Persisted / incremental | 3.220 s | 3.301 s |

App reductions against full were approximately 44.6% same-process and 40.9%
persisted. All 24 rescan requests were incremental with positive reuse, no fallback
and no speculation. All timed or exported scans had 999,110 entries, zero errors/exclusions,
complete coverage and matching deduplicated allocated totals. Eight exports
matched exactly within each layer and between native/app full references. All fresh
open-attempt counts equaled processed directory tasks minus the held initial root.
All timed scans used three workers; source/helper/binary fingerprints stayed unchanged.

**These micro-optimizations do not establish an additional speed gain over the
previous cohort.** They remove duplicate name allocation and unnecessary retained
leaf leases, but required fresh opens dominate. The latest median persisted app run
was slower than the earlier cohort; different datasets and sample variation prevent
a controlled attribution. Earlier observations are retained separately. No fresh
validation was skipped to make timings improve. Results apply to this unchanged
folder/host; changed scopes keep conservative refresh and retries.

Portable results are in `docs/benchmark-results-github-cached-traversal-summary.json`.
Raw observations, baseline profile, state, hashes and eight exports remain under
`/private/tmp/hahakigami-github-cached-traversal-e947gbl2/`.


## Bounded cached validation concurrency (2026-10-13)

A public-journal-authorized whole-GitHub worker probe rotated 1, 2, 3, 4, 6 and
8 explicit core workers, with an initial observation and three warm samples each.
Warm validation medians were 2.780, 1.947, 1.716, 1.635, 2.031 and 2.629 seconds.
This probe excludes journal/cache IO, launch and publication; it supports a bounded
four-worker target, not a claim about complete app timing. Aggregate parity and
fresh-open counts passed; final exact metadata verification was performed below.

Healthy folder scans retain normal hardware Auto for full scans, while valid Unix
seeds with at least 4,096 reusable directory listings can use four validation workers,
bounded by active/total CPUs and MAX_WORKERS and never below the base. Non-nominal
thermal states, low power, volume scans and unknown thermal conditions retain the
base policy. Explicit existing core API counts are preserved. Full fallbacks and
new-hardlink retries restore the base count. Every fresh open, no-follow rule and
identity check remains mandatory; descriptor budgets and invalidation are unchanged.

Both signed builds verified. The normal designated requirement stayed unchanged;
user sessions and privacy grants were preserved. Isolated executables, state, logs
and exports were outside the measured folder. The unchanged-source/binary cohort
used the whole ~/Documents/GitHub with no exclusions: initial plus five warm rounds
of rotating full, same-process and persisted modes in each executable. Launch,
seeding and coverage export are excluded; scan timing includes replay, decoding,
reconciliation, publication and final CPU draw. OS caches were not purged.

| Executable | Full (3 workers) | Memory cache (4 workers) | Persisted cache (4 workers) |
| --- | ---: | ---: | ---: |
| Native | 5.526 s | 2.648 s | 2.849 s |
| Signed app | 5.773 s | 2.568 s | 2.868 s |

These are five-warm-sample medians. App rescans used 55.5% and 50.3% less time than
this candidate's full scans. All 24 timed rescans were incremental, with no fallback
or speculative reconstruction and 865,528–868,208 reused entries. Journal scope
changed the reuse count during the cohort; every observation was retained.

All 36 timed runs and eight exports had 1,000,670 entries, zero errors/exclusions,
complete coverage, 248,790,474,752 deduplicated
allocated bytes. Exact within/cross-layer path, kind, identity and own-metadata
comparisons passed. Fresh relative/absolute open attempts matched every non-root
processed directory, and reported pool counts matched worker rows throughout.

The prior app cohort had 5.586/3.095/3.301-second medians, but differs in data and
conditions. This is not a controlled causal estimate of the policy's speed gain.
Portable evidence: benchmark-results-github-concurrency-summary.json. Full local
report and worker probe: /private/tmp/hahakigami-github-concurrency-hr4u19wv/.


## Private cache encoding and publication (2026-10-13)

Private caches now use lossless Zstd level -1 and a bounded 128 KiB input buffer,
and omit the unused selection write. The encoded node format, checksum validation,
node/decoded/compressed byte limits and archive/row/cursor publication order are
unchanged. Archive frame completion, flush, file sync, atomic rename and directory
sync remain inside scan timing. No work moved into deferred background publication.

A release compression probe loaded the prior actual million-entry private snapshot
once outside the timer. Six rounds rotated levels 1, -1 and -3. Each checksummed
frame was fully decompressed after timing and compared byte-for-byte with all
236,757,031 encoded bytes. Warm medians (rounds 1–5): 204.461, 179.653 and
173.371 ms; archive sizes: 27,458,875, 30,938,136 and 36,004,676 bytes. Level -1
reduces encoding/compression time by 12.1% for 12.7% more storage. Level -3 saves
only another 6.3 ms with materially more storage, so -1 was selected. The existing
512 MiB total cache budget remains enforced. This probe excludes disk IO, validation
and durability. The input-buffer-only gain was within noise (204.084 vs 203.559 ms);
manual staging was slower and rejected rather than shipped.

Signed normal/isolated builds verified; the normal designated requirement stayed
unchanged. Final unchanged-source/binary whole-GitHub cohort: initial plus five warm
rounds rotating full, memory and persisted modes in each executable. State, builds,
logs and exports were outside the measured folder. No exclusions or OS cache purge.
Launch, seeding and coverage export are excluded; scan through publication and final
CPU draw is measured.

| Executable | Full, 3 workers | Memory cache, 4 workers | Persisted cache, 4 workers |
| --- | ---: | ---: | ---: |
| Native | 5.507 s | 2.599 s | 2.840 s |
| Signed app | 5.709 s | 3.120 s | 3.217 s |

| Cache-publication phase median | Full | Memory cache | Persisted cache |
| --- | ---: | ---: | ---: |
| Native | 310.480 ms | 315.979 ms | 313.947 ms |
| Signed app | 321.736 ms | 324.710 ms | 330.069 ms |

These are five-warm-sample medians. App incremental scans took 45.4% and 43.7% less
time than this candidate's full scans. All 24 timed rescans were incremental with
866,874–868,375 reused entries, no fallback and no speculative reconstruction.
All 36 timed observations and eight coverage exports had 1,003,125 entries,
zero errors/exclusions and complete coverage. Exact within/cross-layer path, kind,
identity and own-metadata comparisons passed. Root totals were 249,311,518,720 deduplicated allocated bytes. Every non-root
directory retained its fresh open attempt; pool counts matched worker rows.
Actual new cache archives ranged from 31,041,279 to 31,089,114 bytes.

The prior app cohort had 5.773/2.568/2.868-second medians and cache-publication
medians 348.149/356.100/354.112 ms. Publication time is lower in this cohort, but
whole-app rescans were slower and noisier. Data and journal scope changed, so this
is not a controlled causal end-to-end comparison. Only the same-snapshot encoding
probe supports a direct compression-speed claim. All observations were retained.
Portable evidence: benchmark-results-github-publication-summary.json. Full local
reports/probe: /private/tmp/hahakigami-github-publication-9fo1my9v/.


## Profiled incremental enumeration tail (2026-10-13)

The previous slower app cohort was dominated by the scan phase, not cache encoding:
app memory-rescan scan medians rose from 1,936.590 to 2,490.390 ms, while native
remained near 2,018 ms and cache publication decreased. Summed app worker queue
waits rose from 507.847 to 2,087.766 ms. Three workers commonly finished around
1.0–1.3 seconds, waiting roughly 0.6–0.7 seconds for a fourth worker's metadata
work. Boundary observations remained nominal/not-low-power; they do not measure
background load or actual P/E-core scheduling. This evidence does not establish a
cache-compression regression.

A 1 ms process sample of the isolated app's persisted rescan showed 2,467 openat,
970 getattrlistbulk and 35 fstat stack samples across its scan workers. Fresh
getattrlistbulk work was uneven: 192/109/104/565 samples by worker. Counts are
samples, not syscall counts, and profiler timing is not benchmark timing. Dormant
Tokio/UI thread waits were excluded from this worker interpretation. Required fresh
opens remain a large cost; the avoidable enumeration tail was the target.

The incremental queue now prioritizes expensive dirty branches before neutral cached
validation tasks. A temporary bounded eight-byte-per-node cost vector propagates
validated dirty listing child counts to ancestors, then is dropped before staging.
Only positive-cost directory path/identity hints are retained in an immutable shared
map. Exact path-and-identity matches affect ordering only; unmatched/new/replaced
objects have neutral priority. Cached eligibility, every fresh no-follow open and
identity check, alias invalidation, descriptor budgets, cancellation and private
publication remain unchanged. Ordinary full scans and full retries retain LIFO.

A paired comparison alternated the previous signed app and this candidate against
one unchanged live folder: initial plus three warm observations per version/mode,
16 observations total. Separate state and builds were outside the folder; no cache
purge, exclusions, code changes or compilation during measurement. Every rescan was
incremental and both versions had identical aggregate coverage/totals and four
workers. Warm results:

| Paired signed app | Prior | Candidate | Reduction |
| --- | ---: | ---: | ---: |
| Memory cache | 2.709 s | 2.384 s | 12.0% |
| Persisted cache | 3.178 s | 2.509 s | 21.1% |

Paired scan-phase medians fell from 2,115.888/2,430.161 to 1,793.720/1,772.620 ms.
Summed queue-wait medians fell from 1,278.314/2,247.056 to 6.063/7.447 ms. These
support the scheduling-tail diagnosis; three warm samples per version/mode do not
characterize longer-term host variance. Whole-cohort results below provide final
candidate coverage evidence, rather than claiming timing differences from older
unpaired cohorts are controlled.

Final initial-plus-five-warm rotating whole-GitHub cohort, with unchanged source and
binary fingerprints:

| Executable | Full, 3 workers | Memory cache, 4 workers | Persisted cache, 4 workers |
| --- | ---: | ---: | ---: |
| Native | 5.478 s | 2.294 s | 2.484 s |
| Signed app | 5.591 s | 2.376 s | 2.506 s |

App rescans used 57.5% and 55.2% less time than this candidate's full scans.
Final incremental worker queue waits summed to approximately 6–7 ms. Scan timing
includes journal, decoding, reconciliation, durable cache publication and final CPU
draw; launch, seeding and coverage export are excluded. OS caches were warm.
All 24 timed rescans were genuinely incremental with 868,371–868,388 reused entries,
no fallback or speculative reconstruction. All 36 timed runs and eight exports had
1,004,020 entries, zero errors/exclusions and complete coverage. Exact within/cross-
layer path, kind, identity and own-metadata comparisons passed. Root totals matched:
249,415,540,736 deduplicated allocated bytes.
Fresh-open attempts matched every non-root processed directory; worker counts matched
rows. All paired observations also matched those totals and open/reporting checks.

Portable evidence: benchmark-results-github-tail-summary.json. Full report, paired
comparison and process sample: /private/tmp/hahakigami-github-tail-r3oeossw/.


## Profiled serial seed preparation (2026-10-13)

After removing the worker tail, the previous app scan phase was about 1,757–1,769 ms
while the pool finished around 1,124–1,133 ms and aggregation took 28–32 ms. The
per-run subtraction leaves a median 608–609 ms serial residual. This is diagnostic,
not a separately instrumented constructor phase: it includes setup, joins and final
candidate publication. A 1 ms isolated persisted-app sample showed 227 Path::hash,
195 hasher-write and 104 path-comparison top-of-stack samples, with substantial
Reuse::new work. Required filesystem checks still dominate workers (2,501 openat
and 990 bulk-metadata samples); counts are samples, not syscall counts. Dormant
Tokio/UI waits were excluded and sampled timings are not benchmark timings.

The candidate changes only incremental seed preparation. Validated unique parents,
full child-parent path equality and raw filenames allow uniqueness to be checked by
(parent ID, filename), avoiding repeated full ancestor hashing. Missing filenames
fail closed; dense IDs, kind, connectivity and full parent-path checks remain.
Duplicate parent records and normalized duplicate files/directories still fail.
Dirty subtree/listing/direct-listing categories share one Path-keyed lookup per node;
requested/matched markers preserve unknown/transient-path all-alias refresh. Original
normalized Path equality and ancestor walk remain. Fresh no-follow opens, identity
checks, scheduling, descriptor budget, invalidation policy and publication are
unchanged. No cache format, public API or dependency change was introduced.

A release validation-only synthetic probe used one root plus one million files
under a long prefix, with setup excluded and strategy order alternated for six rounds.
Warm five-sample medians: full-path uniqueness 103.300 ms, sibling-key 56.036 ms
(45.8% lower). It does not measure dirty-map construction, filesystem work or total
scan time. Raw samples from the tool run are retained in the portable evidence;
no raw filesystem log was captured for this microbenchmark.

A paired previous/candidate signed-app comparison used initial plus five warm
observations per version/mode, alternating order on the same live folder. Separate
fresh state and artifacts were outside the scope; no compilation/source changes or
OS cache purge during measurement. All 24 observations were incremental, four
workers each, with equal entries/totals and valid fresh-open counts. Warm medians:

| Paired signed app | Prior | Candidate | Reduction |
| --- | ---: | ---: | ---: |
| Memory cache | 2.368 s | 2.126 s | 10.2% |
| Persisted cache | 2.530 s | 2.386 s | 5.7% |

Paired scan-phase medians: 1,748.326/1,787.707 → 1,545.397/1,613.987 ms.
Diagnostic serial residual: 605.564/612.765 → 384.323/389.037 ms. The latter supports
reduced preparation work but is not an independently timed phase. Five warm samples
per version/mode do not characterize longer-term host variance.

Final unchanged-source/binary whole-GitHub cohort used initial plus five warm rounds
rotating full/memory/persisted modes in each executable. Launch, seeding and coverage
export are excluded; measured scans include replay, decoding, reconciliation, durable
cache publication and final CPU draw. OS caches were warm; no exclusions.

| Executable | Full, 3 workers | Memory cache, 4 workers | Persisted cache, 4 workers |
| --- | ---: | ---: | ---: |
| Native | 5.496 s | 2.117 s | 2.290 s |
| Signed app | 5.615 s | 2.226 s | 2.309 s |

App incremental scans used 60.4% and 58.9% less time than this candidate's full scans.
All 24 timed rescans were incremental with 868,411 reused entries, no fallback and
no speculative reconstruction. All 36 observations and eight exports had 1,005,627
entries, zero errors/exclusions and complete coverage. Exact within/cross-layer
path, kind, identity and own-metadata comparisons passed. Root totals matched:
249,530,667,008 deduplicated allocated bytes. Fresh-open
attempts matched every non-root processed directory; pool counts matched worker rows.
Paired observations matched the same totals and safety telemetry. Final app serial
residual medians stayed near 384–386 ms.

Portable evidence: benchmark-results-github-seed-summary.json. Local sample, paired
comparison and final report: /private/tmp/hahakigami-github-seed-bcg1j8wu/.


## Profiled alias-resolution path sets (2026-10-13)

The remaining cache-check work included a large stable path sort after historical
replay and cached-inode expansion. A 1 ms isolated app sample recorded 142 scan-thread
samples in that sort, plus 90 path-comparison top-of-stack samples. Required worker
filesystem work remained (2,369 openat and 913 bulk-metadata samples). These are
stack counts, not syscall counts or separately timed phase durations; dormant
Tokio/UI waits were excluded from worker interpretation.

Resolved event paths are consumed as sets: speculative selection checks emptiness,
and reconciliation combines commutative path/listing/subtree marks. Standard Path
hash dedup now retains the first original native-byte representative, matching the
previous stable sort/dedup treatment of equal paths. Borrowed keys are dropped before
vector compaction; separate path/listing categories remain separate. Alias selection,
root-device matching, uncertain-alias expansion, FSEvents classification/history/
epoch/cancellation limits, fresh opens and identity checks are unchanged. Scratch
memory is linear in the input vectors. The native replay's existing final sort is
unchanged. A benchmark-only alias_resolution span measures the new phase directly.

A controlled release resolver experiment loaded the previous actual 1,005,627-entry
cache once outside timing. It alternated previous/candidate resolvers for initial
plus five warm pairs per case. Changes cloning/setup were excluded. Inputs selected
the first 100,000 cached inode IDs, then the same IDs with uncertain_aliases enabled:

| Resolver input | Previous | Candidate | Unique resolved paths |
| --- | ---: | ---: | ---: |
| Shared IDs | 183.093 ms | 29.947 ms | 139,171 |
| Shared IDs + uncertain aliases | 262.298 ms | 31.444 ms | 188,313 |

Every pair matched exact normalized Path sets and first representative bytes in
both categories. This is resolver timing on specified event sets, not a full scan.

The paired previous/candidate signed-app comparison used initial plus five warm
observations per version/mode (24 total), alternating order on one unchanged live
folder. Separate state/builds were outside the folder; no compilation/source changes,
exclusions or OS cache purge during measurement. All observations were incremental
with four workers, identical entries/totals and verified fresh-open counts:

| Paired signed app | Prior | Candidate | Reduction |
| --- | ---: | ---: | ---: |
| Memory cache | 2.111 s | 1.963 s | 7.0% |
| Persisted cache | 2.345 s | 2.183 s | 6.9% |

Candidate alias-resolution warm medians were 19.716/20.398 ms. The previous app did
not separately instrument that phase; its paired comparison uses total timing. Five
warm samples per version/mode do not characterize longer-term host variance.

Final initial-plus-five-warm rotating whole-GitHub cohort, with unchanged source and
binary fingerprints:

| Executable | Full, 3 workers | Memory cache, 4 workers | Persisted cache, 4 workers |
| --- | ---: | ---: | ---: |
| Native | 5.579 s | 1.936 s | 2.153 s |
| Signed app | 5.572 s | 1.984 s | 2.139 s |

App incremental scans used 64.4% and 61.6% less time than this candidate's full scans.
Final app alias-resolution medians: 23.188/19.512 ms. Scan timing includes replay,
decoding, reconciliation, durable cache publication and final CPU draw; launch,
seeding and coverage export are excluded. OS caches were warm.
All 24 timed rescans were incremental, with 868,422 reused entries and no fallback
or speculative reconstruction. All 36 runs and eight exact exports had 1,006,388
entries, zero errors/exclusions and complete within/cross-layer path, kind, identity
and own-metadata parity. Root totals matched: 249,730,940,928 deduplicated allocated bytes. Every non-root processed directory
retained its fresh open attempt, and worker counts matched rows. Paired observations
matched these same totals and safety telemetry.

Portable evidence: benchmark-results-github-alias-set-summary.json. Local process
sample, resolver experiment, paired and final reports:
/private/tmp/hahakigami-github-alias-set-r3xtm3mb/.


## Profiled validation/encoding publication overlap (2026-10-13)

After alias deduplication, private cache publication remained about 333 ms in app
rescans. An isolated 1 ms app sample showed pure snapshot validation followed by
encoding/compression in the cache-save stack (72 samples in the validation branch).
Required worker work remained (2,260 openat and 1,478 bulk-metadata top samples).
These are diagnostic stack counts, not syscall counts or exact phase durations.

Large private compact caches (at least 4,096 nodes, captured workers greater than
one) now run full snapshot validation on one scoped auxiliary thread while the
calling thread encodes, finishes the checksummed frame, flushes and syncs the file.
The validator is joined, and BOTH results must succeed before the atomic archive
writer returns and can rename. Metadata/cursor publication follows unchanged. No
validation or durability is omitted, and no work continues past save_cache return.
The immutable Snapshot is borrowed without cloning. The validator takes no locks,
files or callbacks. Encoding failure still joins, validator error/panic rejects the
archive, and thread creation failure uses sequential validation before encoding.
Smaller caches and single-worker sessions retain sequential
behavior. Publication uses at most two threads, within the captured scan-worker
count; scan traversal/pools/fresh opens/identity checks are unchanged.

A release durable-save probe loaded one previous real immutable cache snapshot and
metadata outside timing, then alternated sequential/parallel save_cache into fresh
private directories: initial plus five warm pairs (12 saves). Encoding, file sync,
atomic archive rename/directory sync and metadata publication were included; load
and output verification were excluded. Warm medians: 290.327→224.096 ms, 22.8%
less time. All compressed outputs were byte-identical and loaded/validated. This
isolates archive+metadata saving, not the outer cache maintenance/cursor operations.

Paired old/new signed apps: initial plus five warm observations per version/mode,
24 total, alternating on one live folder with separate state/artifacts outside it.
Source/binaries stayed unchanged; no compilation, exclusions or OS cache purge.

| Paired signed app | Prior | Candidate | Observed reduction |
| --- | ---: | ---: | ---: |
| Memory cache | 2.074 s | 1.974 s | 4.8% |
| Persisted cache | 2.161 s | 2.142 s | 0.9% |

Publication medians fell 328.763/330.551→249.024/253.153 ms (about 24%). Persisted
scan-phase variation (1,559.369→1,613.276 ms) offset much of that saving. The small
persisted total difference should not be treated as a robust broad speed guarantee.
All warm pairs had matching entries/totals and verified fresh-open/reporting counts.
One qualification is retained: initial prior same-process observation allocated
250,477,301,760 bytes, 2,506,752 above the later 250,474,795,008 total. Its cause
was not determined; no observations were discarded. Initial observations were
excluded from timing medians by the method declared before running. All 20 warm
paired observations and final runs/exports agreed exactly on totals.

Final initial-plus-five-warm rotating whole-GitHub cohort, unchanged source/binaries:

| Executable | Full, 3 workers | Memory cache, 4 workers | Persisted cache, 4 workers |
| --- | ---: | ---: | ---: |
| Native | 5.470 s | 1.942 s | 2.092 s |
| Signed app | 5.563 s | 1.945 s | 2.133 s |

Final app rescan publication medians were 252.616/260.396 ms. App incremental scans
used 65.0% and 61.7% less time than this candidate's full scans. Measured scan
includes replay, decoding, reconciliation, durable publication and final CPU draw;
launch, seeding and coverage export are excluded. OS caches were warm.
All 24 final timed rescans were incremental with 869,061 reused entries, no fallback
or speculative reconstruction. All 36 observations and eight exact exports had
1,008,628 entries, zero errors/exclusions and complete within/cross-layer path,
kind, identity and own-metadata parity. Root totals: 250,474,795,008 deduplicated allocated bytes. Fresh opens matched every non-root
processed directory and worker counts matched rows. No work was moved outside timing.

Portable evidence: benchmark-results-github-publication-overlap-summary.json.
Local sample, durable-save probe, paired and final reports:
/private/tmp/hahakigami-github-publication-overlap-be3snokw/.

## Current compact scan speed versus pinned references (14 October 2026)

This separate comparison uses the current accepted 80-byte compact scanner,
Radix `df1235bfb05acb7a678718166491bcc27817d95d` and OpenDisk
`f7551cf21a5c35a963c607401ba5705ec0869b64`. Portable observations, initial values,
all five warm values, ranges, counters, policies and fingerprints are in
the scan-speed summary (`benchmark-results-scan-speed-summary.json`, removed).

Each of the twelve engine/layer/scope cells has one separate initial run and five
consecutive warm runs, rotating the three engines: 72 timed fresh processes,
full scans and fresh state directories. Two independent native exports per
engine/scope before and after measurement add 24 untimed exports. No OS cache
purge, exclusions, tuning or timed reruns were used. All timed runs succeeded with
zero reported errors; every app reported final CPU Canvas draw completion.
All binary/source/adapter/runner/config fingerprints stayed stable through analysis.

Warm medians in milliseconds:

| Scope/layer | Hahakigami | Radix | OpenDisk |
| --- | ---: | ---: | ---: |
| 100,001-entry fixture, native engine | 143 | 519.142 | 139.546 |
| Fixture, packaged app | 218.709 | 880.993 | 191.741 |
| Whole GitHub, native engine | 5,337 | 7,890.905 | 5,142.528 |
| Whole GitHub, packaged app | 5,815.055 | 8,229.122 | 5,374.947 |

These are default-policy workloads, with strict equivalence unverified. Fixture
Hahakigami/reference time ratios are 0.275/0.248 against Radix and 1.025/1.141
against OpenDisk (native/app), describing these policies rather than equal work.
Native retained entry counts were
100,001/66,674/100,001 on the fixture and 1,029,279/877,281/1,029,279 on GitHub
(Hahakigami/Radix/OpenDisk). Radix summarizes packages/eligible directories and
deduplicates APFS clones: 151,998 Hahakigami GitHub paths were not materialized
and 821 shared nodes were summary boundaries. Fixture overlap also had one
Radix-only synthetic path. GitHub allocated totals were 253,187,526,656 bytes for
Hahakigami/OpenDisk and 230,526,128,128 for Radix.
Its app reports a distinct root-excluding progress/storage counter: 100,000 and
1,029,278. No counter normalization was applied.

Fixture exports repeated stably. GitHub export path sets and timed counters were
stable, and Hahakigami/Radix repeat fields agreed. OpenDisk retained the same
1,029,279 paths and stable timed root totals, but its per-path allocated bytes
differed in 356/263 paths between before/after repeats and 335/301 across phases.
The cause is unproven; this could reflect attribution and is not confirmed
filesystem drift. GitHub ratios are therefore
inconclusive; its table retains raw observations without a numeric speed conclusion.
External exports deliberately declare incomplete native-path fidelity, including
unavailable original invalid-UTF-8 bytes. OpenDisk also lacks precise symlink kind
and most identities. Size overlap compares matching own/aggregate scopes only;
identity overlap compares device/inode pairs only. App adapters ignore
`coverage_output`; native overlap does not establish independent app path parity.

Hahakigami native timing is integer `stats.elapsed_ms`, core scan/aggregation only,
with explicit `3 auto --qos automatic` matching the observed desktop Auto bound
rather than the CLI default eight. Native external limits were Radix seven and
OpenDisk three, configured limits rather than observed active counts. External
apps do not report worker bounds. App timing ends at final CPU draw, not GPU
presentation; startup is recorded separately. Native and app times have distinct
boundaries. The compact-memory baseline acceptance gate is not applied here.

An initial restricted-sandbox preflight had zero timed rows and was preserved
separately after Radix waited in LaunchServices XPC. A brief diagnostic sample
was taken only in that untimed preflight. The complete cohort above used host
execution with the same verified binaries/policies and fresh state; no privacy,
TCC, keychain or normal-app-session changes were made. Raw exports and the
preflight incident remain outside GitHub under
`/private/tmp/hahakigami-scan-speed-20261014-bf29edc0/`; the completed cohort is
in `host-run/`, and the preserved failed preflight is labeled
`attempt0-sandbox-preflight/`.
