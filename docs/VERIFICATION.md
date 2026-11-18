# Verification

Paths under `benchmark-results/` are local raw reports; that folder is gitignored, so it is not in a fresh clone. Cited `benchmark-results-*.json` summaries were removed before open-sourcing and remain in git history.

## Six-second gate on the cleaned GitHub scope — 14 October 2026

**Outcome: T3 timing acceptance is met by the unchanged H scanner on the cleaned scope.**

`target/debug` was removed with `cargo clean --profile dev` after confirming it was gitignored build cache with no running processes: 230,886 files. It held a 178,793-entry `deps` listing that set every scan's walk end. `~/Documents/GitHub` went from 1,101,406 to 868,544 entries, and the count was re-verified unchanged after all later builds, which used an external `CARGO_TARGET_DIR`.

The frozen H cohort config was re-run unchanged:
- 72 successful timings, 12 exports, 14/14 exact raw comparisons and 666 stable fingerprints
- every warm GitHub run under 6 s on both layers; H maximums are 4.087 s packaged and 3.963 s native
- H median and sampled RSS below both controls on GitHub
- identical deductions, complete checks and no warnings

Earlier result rows for the larger scope remain valid for that scope.

The only source change is non-persisted tail diagnostics: `Snapshot.enumeration_tail` plus `scan` example output. Workspace/all-target performance-feature checks: **375 Rust tests passed, 22 ignored** (178 core, 189 app, 8 example), with strict Clippy and rustfmt clean. Two new tests cover slow-listing retention and confirm the diagnostics are absent from serialized snapshots.

No app bundle was rebuilt, installed, launched outside the isolated cohort, or re-signed. Privacy grants, keychain and the normal app are unchanged. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-clean-corpus-summary.json`, removed).

## Clone accounting memory and GitHub timing — 14 October 2026

**Outcome: performance acceptance is unmet.** Retained source is the six-worker H production candidate, with the additional I three/eight-worker native safety test only. The eight-worker production policy and its settings tests were restored exactly to H. No further investigation was performed after the user requested the evidence report.

Final restored workspace/all-target performance-feature tests passed **371 tests, 22 ignored**:174 core,189 app and 8 example tests. Strict workspace Clippy, rustfmt and diff checks passed. Unchanged frontend and benchmark tooling retain156 and 105 passing tests. Signed isolated builds include Svelte check/production compilation. Real APFS clone, resource-fork, hard-link, replacement, symlink, cancellation and constrained-descriptor checks passed, including the dedicated 3/8-worker parity/rejection fixture. Non-Unix execution was not run; its original lookup path is preserved by source review.

H's frozen cohort completed 72 successful timings,12 exports, 14 exact native raw comparisons and 666 stable fingerprints. Packaged GitHub median and both memory measures improve against both controls; short-fixture high-water savings hold. However packaged warm3 is **6,147.352708 ms**, so the strict all warm scans under 6 s criterion fails. Native GitHub median is 0.99% slower than latest. Every failed prior cohort/pilot and the rejected eight-worker experiment remain in portable evidence (`benchmark-results-growth-memory-summary.json`, removed), with the limitations and exact rows in [BENCHMARKS.md](BENCHMARKS.md).

Independent reviewer_high source reviews found no remaining blockers in retained H and the I test-only expansion. G1 retained capped-calibration allocation policy; G2 preserved non-Unix path compatibility. Both automatic correction rounds were used. **T3 remains unmet; source approval is not performance or delivery approval.** Final evidence-only Max review approved the report’s accuracy with no blockers. This approves the report only; T3 and normal delivery remain unapproved.

All measured isolated apps were built using `pnpm desktop:build --benchmark-output` with the existing development identity and verified signatures. **No normal packaged update was built or installed**, because the full acceptance gate did not pass. The pre-existing normal app still passes deep/strict verification, its designated requirement and binary hash are unchanged, and its process count remains 0. It was not launched or restarted; privacy grants and keychain settings were unchanged. Full Disk Access remains manually managed.

## Wide-listing scratch and fixture high-water reduction — 14 October 2026

Only scanner `bulk.rs` and `lib.rs` runtime code changed. Large bulk output lists reserve at 1.5× growth after whole-batch validation; calibration allowance caps the request. Child-ID scratch is sized from the private listing length and remaining node budget. Parser masks/length/identity validation, clone revalidation, cancellation, order/ordinal sidecars, publication, 80-byte records, archives, caches, UI, scheduling and cleanup authority are unchanged.

Final workspace/all-target performance-feature tests: **362 Rust tests passed, 22 ignored** (165 core, 189 app, 8 example tests). **156 frontend tests** and **105 Python tests** passed. Strict workspace/all-target performance-feature Clippy, formatting/diff checks, Svelte checking (0 errors/0 warnings), production frontend build and isolated signed app build passed. New tests cover wide-listing capacity/order and calibration limits; existing malformed/recovery/cancellation/real-APFS tests pass. Max source review found no blockers and used zero correction rounds.

The frozen cohort completed 72 successful timings, 12 complete exports, all 14 exact raw comparisons and 953 unchanged fingerprints. Fixture native app high-water falls **5.05% (135,168,000→128,335,872 bytes)**, with every candidate warm observation below every baseline observation. All candidate warm native/app scans remain under six seconds; GitHub packaged maximum **5.885 seconds**. Large-scope sampled packaged savings remain **13.29% versus pre-chunk**, with a 0.44% increase versus the latest baseline. GitHub packaged/native medians are 4.33%/1.19% slower than latest baseline. Deduction parity, complete accounting checks, empty warnings and populated map geometry pass. Timing margin, small large-scope memory increases, sampling limits and non-atomic observations remain explicit in [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-listing-memory-summary.json`, removed). UI source is unchanged; prior functional smoke evidence is reused without repeating its uninstrumented slow-scroll claim.

Normal `pnpm desktop:build` passed with the existing development identity. Host deep/strict signature verification passed; the designated requirement exactly matches the pre-build value. Normal app process records remain unchanged (zero processes), without an app launch/restart or privacy/keychain changes. Post-cohort runtime/helper source hashes still match. Final evidence-only `reviewer_high` review closed T2 and approved delivery with zero correction rounds. It verified the high-water reduction, all warm timing and packaged-savings gates, coverage/accounting parity, unchanged hashes and signing/session preservation, retaining the latest-baseline timing/memory tradeoffs and limited timing margin.


## Warm-scan limit and empty-canvas allocation — 14 October 2026

Empty maps now retain no canvas backing store or 2D context; populated maps still draw at full DPR. Six folder workers require known active/hardware counts ≥14, nominal thermal state and low-power mode off. Core clone/parser/accounting/archive/chunk code is unchanged from the prior reviewed candidate. Safety checks, zero-attributed selectable files, native paths, and cleanup authority remain unchanged.

Final workspace/all-target `performance-benchmark` tests passed: **360 Rust tests, 22 ignored** (163 core, 189 app, 8 example tests). **156 frontend tests** and **105 Python tests** passed. Strict workspace/all-target performance-feature Clippy, formatting/diff checks, Svelte check (0 errors / 0 warnings), and isolated production frontend/signed app build passed. Tests cover empty/positive/stale map completions, backing release/reallocation, selection/resize/DPR, healthy/missing/mixed/pressure CPU states, and hostile geometry observations. The independent Max source review found no blockers and required zero corrections.

Frozen measurement completed 72 successful timings, 12 complete native exports, 14 exact raw comparisons and 1,030 stable fingerprints, with no timed retries/exclusions or concurrent builds/tests/edits. All candidate warm scans are below 6 s; GitHub packaged maximum 5.463 s. Packaged sampled memory retains 11.02% savings versus pre-chunk and falls 0.83% versus the latest chunked build. Fixture sampled memory falls 0.42% versus latest, while native appHWM rises 0.56%; the old 13 MB gap does not recur and its cause remains unproven. Native GitHub RSS rises with six workers. These mixed limits remain explicit. Deduction parity and accounting warnings/checks pass across all warm roles. Candidate app geometry verifies full-resolution backing for every timed observation, without a sampling/timer change; it does not prove GPU presentation. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-tail-memory-summary.json`, removed).

Normal `pnpm desktop:build` passed, followed by host deep/strict signature verification. The designated requirement exactly matches the pre-build value; normal app process records remain unchanged (zero processes), with no launch/restart or privacy/keychain changes. The isolated packaged app passed all nine non-destructive interaction phases and a separate CUA screenshot showed a populated clone map. The smoke’s unusually slow scrolling remains in the evidence and is not a scan-performance measurement. Final evidence-only `reviewer_high` review closed T1 and approved delivery with zero correction rounds. It verified all measurement/signing gates and retained the mixed fixture-memory, unchanged packaged median, unproven causal/future timing improvement, and slow interaction-smoke qualifications.


## Chunked record storage and measured packaged RSS — 14 October 2026

The private record store uses safe 4,096-record chunks with unchanged 80-byte records, IDs, native paths, adjacency and clone metadata. Full chunks remain in place during append; deep clones own separate payload and handle partial-chunk append safely. Index/get bounds, mutation, paths and archive identity contracts remain checked. Chunk payload and outer-header buffers are charged separately; benchmark contracts reject explicit malformed geometry while supporting older omitted fields. No scanner/parser, clone-accounting, no-follow/identity/cancellation, archive/cache format or action-authority rule changed.

**347 Rust tests passed**, 22 opt-in ignored; 154 frontend and 104 Python tests passed. Checks include >1M growth, 4096/8192 boundaries, partial deep-clone mutation/append, native invalid-UTF paths/children/metadata, malformed diagnostic markers, and a 9,000-node v2 clone-attribution check across chunks. Strict workspace all-target performance-feature Clippy, formatting/diff, Svelte checks and production frontend build passed; host APFS fixtures ran. Max source review closed C1 after one correction round. Other OS/Intel hardware remains unexecuted.

Final evidence-only Max review approved this incremental candidate and closed C1, retaining the 6.053-second tail, slightly slower native median and mixed short-fixture RSS qualifications.

Final `pnpm desktop:build` passed with the existing development identity. Host deep/strict signature verification passed and the designated requirement exactly matches the pre-build bundle. No normal app was launched/restarted; privacy and keychain settings were unchanged.

The frozen five-round comparison completed 72 timings/12 exports/910 stable fingerprints, with no concurrent builds or edits. Large-scope packaged sampled RSS fell 373.4→333.3 MB (10.73%), native app high-water 375.6→336.2MB. Packaged median 5.51 s is below6s, but one warm run reached 6.053 s; no strict maximum claim is made. Short-fixture sampled RSS increased despite lower native high-water; all rows remain. All 14 raw comparisons and every warm same-model deduction match. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-chunk-memory-summary.json`, removed).

## Packaged memory and healthy-host scheduling — 14 October 2026

Chart keys/cache IDs exclude zero-selected-size nodes without changing visible map order, equal-key fallback, aggregates or list/search membership. Canvas backing dimensions change only with actual pixel dimensions; absolute transforms preserve selection, resize and DPR redraws. Split sparse probe queues retain the original observation allocation and the exact visited cancellation prefix. No clone eligibility, raw/native accounting, ancestry/identity check, archive/cache authority or cleanup rule changes.

Folder worker selection uses the existing five-worker upper bound only with known active CPU>=12, sufficient hardware threads, nominal thermal state and no low-power mode. Unknown evidence and pressure limits remain conservative; all current bounds and descriptor checks remain. One independent Max-review finding (M1) required explicit active-CPU evidence; the missing-reading regression now selects 3. Corrected workspace tests passed **342 Rust tests**, 22 ignored; **154 frontend** and **104 Python** tests passed, along with strict workspace all-target performance-feature Clippy, fmt/diff, Svelte checks and production frontend build. Focused tests cover map JSON/order/cutoff parity, invalid-native/repeated-key ties, 32K zero-child scratch, canvas assignment/transform counts and queue allocation/order/cancellation prefixes. Real APFS tests ran. Non-macOS/Intel/other CPU hardware were not executed.

Final evidence-only Max review closed M1 and approved this incremental candidate, explicitly retaining the near-flat packaged RSS result and single-host performance limit.

Final `pnpm desktop:build` passed using the existing development identity. Host deep/strict signature verification passed and the designated requirement exactly matches the pre-build bundle. No normal app was launched/restarted; privacy and keychain settings were unchanged.

The first measurement attempt is explicitly incomplete (37 timings/6 exports) and preserved. The corrected five-round cohort completed 72 successful timings/12 exports/905 stable fingerprints without concurrent builds or edits. GitHub app median 7.294→5.574s; actual worker policy 3→5 contributes to this result. Native fixture memory falls; packaged sampled/native RSS is near-flat rather than a proved process-memory improvement. Fixture map scratch/cache shrink to 72/12 bytes and canvas tests prove avoided backing resets. All clone deductions match, and fixture/final GitHub raw overlaps are exact; one initial build-artifact raw-allocation drift remains a failed comparison. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-app-memory-summary.json`, removed).

## Singleton-pruned clone validation — 14 October 2026

After all-file hard-link alias consistency checks, a group with one distinct in-scope inode retains raw allocation and unverified metadata without a final query. It cannot authorize a clone deduction or verified owner attribution. Every path/alias in a multi-inode group still receives the prior ancestry/file/clone checks, with whole-group failure and cancellation handling. Nonempty clone metadata independently prohibits incremental reuse; raw singleton archives round-trip without deduction or owner attribution. Stable owners, raw/native data, historical accounting models and cleanup authority remain unchanged.

**338 Rust tests passed** (159 scanner + 179 app), 22 opt-in tests ignored; 153 frontend and 104 Python tests passed. Host APFS tests include four-to-seven-member dense groups, outside-scope clone plus hard links, no-query singleton/repeated aggregation/cancellation, separate-device IDs, late alias replacement, scratch reuse after ancestry failure, and unverified singleton v2 archive round-trip. Strict workspace all-target performance-feature Clippy, formatting/diff checks, Svelte checks and production frontend build passed.

Independent `reviewer_high` source review required one correction: private `ClaimStatus::Raw` is constructed only on macOS, so its declaration is now gated there to avoid non-macOS dead-code failure under strict lint. The correction was applied after fingerprints closed. Measured/final source hashes remain separate; the rebuilt macOS scanner is byte-identical to its measured binary. 33 native clone tests and strict workspace Clippy re-passed. No non-macOS target is installed; full non-macOS/Intel builds remain unexecuted. Final evidence-only Max review closed V1 and approved this incremental candidate, retaining the unmet six-second target and the primary cohort’s packaged-fixture RSS increase.

Final corrected `pnpm desktop:build` passed with the existing development identity. Host deep/strict signature verification passed, and the designated requirement exactly matches the pre-build bundle. No normal app was launched/restarted; privacy and keychain settings were unchanged.

Five rotating warm rounds retained 72 successful timings, 12 complete native exports and 903 stable fingerprints. App median 9.216→7.416s; validation 3.091→1.044s. Six seconds remains unmet. Fixture native memory stays flat; packaged-fixture sampled RSS increased in the full cohort, while a separate retained paired diagnostic reversed direction amid wide variation. No consistent app-fixture memory claim is made. Final raw overlaps and all fixture comparisons match; earlier target/debug allocation drift remains failed comparisons. All warm same-model clone deductions match. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-clone-validation-summary.json`, removed).

## Sparse clone listings and balanced validation — 14 October 2026

The temporary listing record returns to 72 bytes using sparse ordinal clone observations. Enumeration workspaces are reused, native names are borrowed, and final validation schedules complete parent ranges with large tasks first using the original worker bound. Regression tests cover sidecar alignment/rollback, dirty buffer tails, cancellation, whole-parent query selection, disjoint task/status reconstruction, bad-member group invalidation and ancestor substitution between tasks. Every task resets current-parent state so ancestry is checked again. No archive, pure accounting or action-authority rules change.

**333 Rust tests passed**, 22 opt-in tests ignored; **153 frontend tests** and **104 Python tests** passed. Real APFS full-clone/resource-fork/broken-sharing fixtures ran on this host. Workspace formatting/diff checks, strict workspace all-target `performance-benchmark` Clippy, Svelte checking and production frontend build passed. Independent `reviewer_high` source audit found no blocking defects and requested no corrections. Final evidence-only review approved this incremental candidate, retaining the unmet six-second target and live-filesystem qualifications. Other operating systems/Intel were not executed.

Normal `pnpm desktop:build` passed with the existing development identity. Host deep/strict signature verification passed; the designated requirement exactly matched the pre-build bundle. No normal app process was launched/restarted, and no privacy or keychain settings changed.

Five rotating warm rounds completed with 72 successful timings, 12 complete exports and 902 stable fingerprints. Flat-fixture native RSS fell 15.74%; GitHub native time fell 3.18%. App time improved only 0.97%; six seconds remains unmet. Final native cross-role raw overlap is exact; earlier build-artifact allocation drift and external repository changes remain explicit failed comparisons in the evidence. Deduction parity holds for all warm same-model rows. See [BENCHMARKS.md](BENCHMARKS.md) and portable evidence (`benchmark-results-clone-listing-summary.json`, removed).

## Clone-accounting optimization — 14 October 2026

The optimized scanner integrates initial clone metadata into held-descriptor enumeration, streams dense final listings, and retains individual checks for small/sparse groups. Final validation uses the existing bounded worker count with disjoint status slices, fresh root/cached-ancestor checks on parent switches, independently reopened bulk cursors and global group rejection after joining. Cancellation clears all verification. Missing/unsupported fields use the original per-file reader; malformed/changed evidence keeps raw allocation. Sparse probe ordinals and a single pure accounting plan reduce temporary copies; the 80-byte record, archival models and cleanup authority remain unchanged.

The required Max review used two automatic correction rounds. R1 restored raw listings and separate warnings after malformed rich queries, including empty/failing fallback and earlier permanent errors. R2 included retained read workspaces in payload diagnostics, including later direct/failing parents. The stable source received no remaining code findings; the final evidence-only Max review approved delivery with the live-scope qualifications retained. Regressions cover native bulk/direct/missing/unsupported equivalence, omitted/duplicate/malformed records, real resource forks, 4,204 APFS clones with cross-worker group rejection/cancellation, sparse preliminary ordinals, adaptive query choice, bounded parent reuse and ancestor/symlink replacement.

Final checks: **329 Rust tests passed**, 22 opt-in tests ignored; **153 unchanged frontend tests** and **104 Python tests** passed; workspace formatting/diff checks, strict all-target performance-feature Clippy, Svelte checking and production frontend build passed. Native APFS support was exercised on this host. Other macOS/Intel/non-macOS platforms were not executed. Packaged validation used the isolated signed app for all benchmark observations; the normal app uses `pnpm desktop:build` and the existing development identity. Deep/strict verification passed and the designated requirement exactly matched the pre-build bundle; no normal app was restarted and no privacy/keychain settings changed.

Frozen measurement: 72 successful observations, 12 complete exports and unchanged fingerprints. Five warm GitHub medians improve **21.408→9.852 s native / 21.135→9.902 s packaged**, with RSS **341,934,080→289,800,192 / 420,315,136→388,743,168 bytes**. It still costs more than the 5.668/6.076 s hard-link-only reference; flat-fixture memory rises despite faster scans. Deductions match all same-model warm rows: 4,096 fixture / 22,644,379,648 GitHub bytes. Fixture parity and initial GitHub overlap are exact; later GitHub exports contain live writes in an unrelated repository and are explicitly not exact parity evidence. [Detailed limits](BENCHMARKS.md#apfs-clone-accounting-optimization--14-october-2026), portable summary (`benchmark-results-clone-optimization-summary.json`, removed).

## APFS full-clone accounting — 14 October 2026

New scans retain raw allocation while assigning verified shared data-fork allocation once per distinct inode group. Native probes use held directory descriptors; final revalidation walks from the original held root. Resource-fork allocation remains per inode, hard links reconcile first, and partial/unknown sharing retains raw allocation. The 80-byte compact record is unchanged; clone evidence and inspector attribution are sparse. Snapshot summaries distinguish accounting warnings from filesystem coverage errors.

Optimization cache/cursor version 3 requires explicit nonshared evidence. Clone-bearing, uncertain, old-format and newly shared candidates require full scans; speculative topology stays private through the bounded retry.

The installed SDK's volume clone-mapping flag underreports support on this host. Successful complete clone-attribute queries establish support; uncertain responses never authorize deductions. This behavior was verified on real disposable APFS clones with both bulk and portable readers, distinct resource forks, hard-link aliases and writes breaking full sharing. No file contents are read by the production scanner.

Review required an explicit clone/uncertainty private-retry regression and complete grouping-buffer payload estimates. It also identified a version-2 duplicate-inode/false-hard-link-flag accounting defect; contradictory evidence is rejected when cache records load, and native link-count consistency/revalidation drops affected groups to raw allocation. The corrected stable candidate was approved by the same Max reviewer after two correction rounds, with no remaining blockers. A subsequent bounded parent-handle optimization received its own focused Max approval: it retains original held-directory authority, performs every file query, drops the old cached handle before switching, and never reuses it after a failed walk. Tests cover consecutive reuse, directory/symlink replacement during a group and fresh ancestry validation after switching away/back.

Final checks passed: **312 Rust tests** (22 opt-in tests ignored), **153 frontend tests**, **103 Python tests**, workspace formatting/diff checks, strict Clippy including performance-benchmark/all targets, Svelte checking with zero errors/warnings, and production frontend/native builds. Native fixture tests passed on this host; older macOS/Intel/non-macOS targets were not executed. Initial restricted-sandbox workspace failures concerned existing disposable Home fixtures/public FSEvents checks; the host run passed without changing privacy settings.

The final isolated signed app rendered clone-aware results and verified all nine non-destructive explorer interaction phases. Its 100,001-entry fixture showed 4,096 bytes of clone deductions with no accounting warnings. The normal app was built with `pnpm desktop:build`; deep/strict verification passed and its designated requirement exactly matched the pre-change bundle. Neither the normal application nor its in-memory results were relaunched; no TCC, keychain, permission or certificate configuration was changed.

The frozen final comparison completed 48 successful timing observations and eight complete native exports with unchanged fingerprints and all raw path/identity/size repeat/overlap checks matching. GitHub retained 1,046,304 entries while removing 22,643,912,704 bytes of duplicate data attribution. Full-clone accounting remains materially more expensive: **20.840 s native / 20.799 s packaged**, versus **5.395 s / 5.829 s** baseline warm medians, with higher GitHub RSS. The earlier slow candidate is retained separately. [Measured costs and limitations](BENCHMARKS.md#apfs-full-clone-accounting-cost--14-october-2026), portable summary (`benchmark-results-clone-accounting-summary.json`, removed).

## Compact-tree verification — 14 October 2026

The final fixture/GitHub cohort passed all 14 gates: 448 timed observations (one initial plus 15 consecutive rotating warm rounds per cell), 64 complete raw exports, 60 equivalent exact path/kind/identity/own-size/accounting comparisons and 262 unchanged source/binary fingerprints. Every requested incremental timed run/export reused entries without fallback. GitHub full RSS reductions were 58.0% native/49.3% signed app; requested retained capacity fell 61.6%. Worst timing ratio was 1.041879. The earlier Home/startup target remains separate and unmet/inconclusive; see BENCHMARKS.md for all medians, failed attempts and measurement limits.

Core/native compact-store safety and compatibility reviews were independently approved at Max. The final profiled map-key correction received independent `reviewer_high` approval with no findings: `lib.rs` SHA-256 `4f1c6e18daa919edda9b8ed0062b2eb5d4c5b3b4d1672c869ace75b21ded31f9`, `tests.rs` `9dde2caf19e3ea40cbb86658bccd35bb9bb9f57abff2bf929f72851ac10195dc`, `performance.rs` `3bf065f278584202f1ac93bc7ea5a730e70f860903715bed762980814af8dda4`. Unaffected prior review evidence was retained. The correction preserves comparator/tie order and lock policy; temporary map keys are measured separately and discarded. Full identity/all four u64 sizes, held-root/no-follow checks, alias/journal rules, exact legacy V1 JSON/HAHCAC02 compatibility and frontend NodeView contract remain covered.

Final workspace/all-target performance-feature Rust tests: **288 passed, 22 opt-in ignored, 0 failed**. Strict workspace Clippy default and performance-feature configurations passed; cargo fmt and diff checks passed. Frontend **139 tests passed**, framework check **0 errors/0 warnings**, Vite production build passed; Python **99 tests passed**. No further test/source changes were made during final frozen measurements.

Final isolated candidate signed-app smoke: rendered completion, interactions.success and all nine outcomes verified (selection, scrolling, nested expansion, search, sorting, filters, navigation, size metric, map switch). Cleanup deletion was never committed. Normal delivery used the existing development identity through `pnpm desktop:build`; read-only `codesign --verify --deep --strict` passed and the designated requirement matched the pre-change value byte for byte. No normal app launch/restart/kill, TCC/protected grant probes, keychain/certificate changes or privacy-grant changes occurred; existing app sessions were preserved (zero normal processes before bundle replacement).

Portable compact evidence (`benchmark-results-compact-memory-summary.json`, removed) records medians, all arenas/overrides/root/issues/workers, separate temporary-query scope, source/binary hashes, test/review dispositions and exact local artifacts. The final archive is `/private/tmp/hahakigami-compact-memory-20261013-275cafc3/attempt-4-final-15/`; prior failures/inconclusive exports are retained alongside it. This verifies the compact cohort on this Mac, not cold-cache performance, external app equivalence or the older Home/startup acceptance.

Host: Apple Silicon Mac, macOS 26.5, Xcode 26.6, Rust 1.99.0, Node 24.21.0 and pnpm 11.24.0. The application targets macOS 14+, but this run does not certify older macOS versions or Intel hardware.

## Persistent cleanup queue — 14 October 2026

The app-wide queue persists lossless native paths, identities, recorded sizes and failure reasons in private bounded records. Restarted candidates remain inactive until a completed live scan matches their original root, item and ancestor identities. Queued reviews bind the current queue revision and active scan; direct reviews keep their existing authority. Both execute whole-batch and immediate per-item validation before native Trash. Successful moves prune covered candidates; save failures after a successful move are reported separately and invalidate the old scan.

Verification passed: all 149 frontend tests (including ten queue scenarios), `cargo test --workspace`, ten focused Rust queue tests, `pnpm build` with zero Svelte errors/warnings, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo check -p hahakigami --features performance-benchmark`. The opt-in `native_trash_and_recovery` test passed, moving only its disposable fixture and restoring its original contents. Ordinary queue cleanup tests inject outcomes rather than moving user files.

Queue tests cover persistence and native bytes, overlap/capacity limits, corrupt recovery, atomic publication failures, root/item/ancestor replacement, expired/replayed/revision-invalidated reviews, scan replacement and cancellation, partial success, direct cleanup pruning, retained errors and post-Trash persistence warnings. Frontend tests cover navigation/selection preservation, setup-aware actions, readiness, clear-dialog focus/cancellation and failure presentation. Packaged native queue visual inspection is not part of these automated results; the user's running application and in-memory results remain untouched.

Independent safety review identified three correction requirements: report actual successful move counts separately from persistence warnings, display retained errors independently of readiness, and recover nonregular directory records through explicit Clear. Regression tests now cover all three, and the same reviewer approved their closure with no new defects in the corrected scope. Unexpected nonregular records are quarantined without deleting their contents; failed publication restores the original record name. The corrected native application regression suite passed with 171 tests and 20 opt-in tests ignored; the native Trash recovery test was run separately and passed.

Delivered through `pnpm desktop:build` using the existing Apple Development identity. Strict bundle verification passed and the designated requirement matches the pre-build bundle. The build did not launch or replace the running application session, change privacy grants, or notarize/upload the bundle.

## Automated checks

- `cargo test --workspace`: 19 scanner tests and 11 shell tests passed. The native Trash recovery and million-node query benchmarks are opt-in. Native Trash recovery passed separately.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `pnpm test`: 17 tests passed across four files, including stale scan messages, worker layouts, virtualization, tree expansion, navigation/search, cancelled-state cleanup restrictions, confirmation cancellation and original-target rescan.
- `pnpm build`: TypeScript and production Vite build passed.
- Benchmark tooling has four coverage/gate unit tests. Real Rust/OpenDisk engines agree exactly on all five deterministic fixtures: regular files, symlinks, hard links, unusual names and sparse storage.

Scanner regressions exercise checked native record parsing, packed attributes, signed device IDs, descriptor caps, directory/ancestor/root replacement, live hard-link deduplication, permission failures, cancellation, mount policy, and real startup firmlink identity normalization. Shell regressions cover stale IDs, map budgets/pagination, expiring one-use review tickets, protected roots, overlapping selections, concurrent mount discovery and reselecting a replaced target.

## Independent review

A focused independent reviewer checked native parsing/traversal, IPC authority and Trash safety. Findings R1–R5 were corrected; final disposition: **approved, no remaining blocking findings in that scope**. Approval does not establish speed acceptance or exact APFS accounting. Subsequent Clippy edits were equivalent spelling/portability annotations and were checked by the owner with the affected tests.

## Native integration

A built `.app` launched successfully on this host. A real disposable folder scan completed with 156 entries; both Canvas maps rendered, and name search/selection worked. The exact-path Trash confirmation was cancelled without changing the fixture. Confirming a fresh review moved only the disposable file and automatically rescanned the same folder, yielding 155 entries. A separate native Foundation test moved its disposable fixture to Trash, restored it, and verified its contents. Source fixture directories were cleaned up; one disposable GUI test file remains recoverable in Trash.

The reviewed bundle was restarted and scanned the actual project directory: 30,428 entries in 0.1 seconds. Lazy tree expansion displayed the source files correctly. A startup scan showed live maps/tree and cancellation retained 742,718 entries. Cancellation was requested around 2.8 seconds but the final elapsed time was 18.7 seconds; a subsecond stop was not established. In-flight filesystem/permission reads cannot be forcibly interrupted by the cooperative cancellation token. The interface now immediately acknowledges the request with a disabled “Stopping…” control until the native terminal state arrives.

The standalone million-entry in-memory fixture measured 61 ms construction, 6 ms aggregation, 5 ms page sorting and 205,946,880 bytes peak RSS. The explicit release shell-query benchmark passed with 1,000,001 nodes: construction 208 ms, first page 3 ms, cached page below 1 ms, bounded 5,000-entry map 10 ms, and name search 39 ms. Peak RSS was 234,192,896 bytes. These timings exercise the actual backend query code, excluding webview transport/rendering.

## Limits

- Public signing/notarization, universal packaging and other desktop platforms are deferred. Full Disk Access is a manual macOS grant.
- Allocated sizes do not deduplicate shared APFS clone extents or fully attribute snapshots/purgeable space. Startup scope deliberately differs from OpenDisk's inclusion of sibling system volumes.
- Identity/ancestor checks immediately precede native Trash, but the native path-based operation is not atomic with those checks. No permanent-delete fallback is implemented.
- [Benchmark methodology and results](BENCHMARKS.md) retain failed baselines and candidate fingerprints. Equivalent full-disk path sets and direct OpenDisk.app timings remain unverified; an engine-speed win alone is not full acceptance.

The final bundle rebuild succeeded and was relaunched to its ready screen after adding immediate cancellation feedback. The development server was stopped so `pnpm tauri dev` can bind port 1420 normally.

## Full Disk Access setup follow-up

The app now asks for manual Full Disk Access setup before any scan or folder picker. The confirmation is remembered for the executable build; the current process stays gated until relaunch. This is a user acknowledgement, not detection or proof of a macOS grant. Only the app's own configuration and executable metadata are read for setup. Native scans and cleanup serialize with setup/restart using the same gate; a running scan prevents confirmation, and an in-flight Trash operation finishes before setup can require restart.

- `cargo test --workspace`: 19 scanner tests and 16 shell tests passed; two unrelated opt-in tests remained ignored. New checks cover first-run scan rejection, malformed/oversized preferences, failed-save cleanup, persistence across relaunch, executable replacement, and confirmation during a running scan.
- `pnpm test`: 23 tests passed. Setup coverage includes scan/picker suppression, exact app path, Settings shortcut, explicit checkbox, save-before-restart ordering, loading/save/invoke failures, pending restart retry, and stale backend confirmation.
- `pnpm build`, `cargo fmt --all -- --check`, and `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `pnpm tauri build --bundles app`: passed; updated host-architecture bundle launched successfully.
- Native UI checks: initial setup appeared with the actual `.app` path and unchecked confirmation; save/restart was disabled. After Later, both Scan and Choose a folder reopened setup without scanning or opening the picker. The Settings shortcut opened the Full Disk Access page successfully. No macOS access grant or setup acknowledgement was changed by these checks.
- Independent focused review of persistence, native gates, restart/Trash coordination, IPC permissions and frontend flow: approved with no blocking findings.

Automatic OS relaunch after a real manual grant was not exercised. Mocked restart failures establish frontend handling of command rejection, not recovery from an OS relaunch failure after process exit; quit/reopen guidance remains visible in the setup flow. Previous performance acceptance limitations still apply; scanning/benchmark algorithms were unchanged by this follow-up.

## Performance follow-up

The adopted change makes Canvas pointer interaction cheaper: one shape hit test per event, followed by an indexed node lookup, replaces repeated searches and array copies inside the node predicate. Selected IDs use a set for highlighting. Scan buffer reuse, a smaller buffer, and higher worker counts were benchmarked and rejected; final production scanner behavior remains unchanged.

- Final `cargo test --workspace`: **20 scanner and 16 shell tests passed**, two unrelated opt-in tests ignored. Added complete native/portable equality regression for a large directory spanning multiple bulk batches.
- Final `pnpm test`: **27 tests passed across six files**. Added treemap paint-order/edge checks, sunburst ring/angle/hole checks, and a real component fixture with 5,000 nodes checking one canvas measurement per hover/click/double-click, refreshed node indexing, and non-actionable groups.
- Python benchmark tooling: **11 tests passed**. Mutation rejection, incomplete measurements, initial-run exclusion, coverage drift and per-variant worker arguments are covered.
- Production frontend build, formatting, Clippy with warnings denied and local `.app` bundle build: passed.
- Synthetic extracted pointer benchmark: all seven old/new selected IDs matched. Initial and three alternating warm samples retained; excluded DOM/rendering/IPC/scan costs are explicit in the [benchmark report](BENCHMARKS.md).
- Browser Canvas smoke check: the 5,000-entry fixture selected an item in treemap, switched to sunburst, and selected an item in its ring. No filesystem actions were attached to this synthetic fixture.

Both experimental scanner candidates received focused independent review without blocking safety findings before measurement. They were subsequently reverted because performance did not justify adoption. The final native bulk parser matches the saved production baseline; only its regression test coverage increased. Main reviewed the adopted pointer lookup change and its selection/action behavior. Original scan-performance acceptance remains unmet; the map timing improvement does not change that status.

The benchmark CLI reports the production scanner's effective worker bound, preventing unsupported requested counts from masquerading as additional concurrency. A disposable empty-folder smoke check with requested counts 0, 8, and 16 confirmed reports of 1, 8, and 8 respectively, with successful completion and one entry. The rebuilt `.app` was left ready for the next launch while the user's completed in-memory scan session remained open. The temporary browser fixture tab and development server were closed after verification.

## Worker Settings follow-up

Added Explorer/Settings navigation and a persistent worker selector. Rust validates the desktop range against available CPU threads and a shared scanner maximum of 16; the recommended default stays 8 (capped to fewer available threads). The setting is captured per session, included in every scan summary, and cannot alter a running scan. Settings saves block new scans and cleanup until their result is known; cleanup reviews/operations block settings saves so automatic rescanning cannot be skipped.

- Native workspace tests: **40 passed**, two unrelated opt-in tests ignored. Preferences tests cover persistence, CPU/hard caps, invalid/oversized/corrupt records, failed-save rollback and temporary-file cleanup. Native command integration saved a new choice during a running session, preserved that session's worker count, then completed a disposable scan with the saved count.
- Expanded scanner fixture: worker counts 1, 2, 4, 8, 12 and 16 passed with both Portable and Auto enumeration, preserving entries without errors.
- Frontend tests: **34 passed**. New tests cover selection/save/reload, recommended reset, loading/write failures, browser guidance, tab state, immutable active-scan count, pending-save scan blocking, and cleanup/save serialization with successful post-Trash rescan.
- Formatting, Clippy with warnings denied, TypeScript/Vite production build and local `.app` bundle build passed.
- Independent review found R6, a save/cleanup race that could suppress automatic rescan. Handler guards and disabled controls now serialize both operations; the deferred-save/Trash/rescan regression passes. Final review disposition: **approved, no remaining blockers in this scope**.
- Browser visual fixture: selected four workers and saved successfully, with guidance that the active eight-worker scan remains unchanged. This fixture simulated settings commands; native persistence and scan selection are established by Rust tests. No user preference or disk permission was changed through this visual check.
- Current release CLI smoke: requested 0, 8, 16 and 32 workers reported effective counts 1, 8, 16 and 16; each disposable empty-folder scan completed with one entry.

The built bundle is ready for the next launch; the user's running app session was preserved. Increasing the selectable worker maximum is a user-control feature, not a measured speed improvement. Earlier benchmark evidence and its unresolved OpenDisk acceptance limits still apply.

## Worker diagnostics follow-up

Added per-worker contribution/task/issue counts, processing/lifetime/CPU time, metadata/store/commit/dispatch/progress phase times, sampled requested QoS, and relative/absolute directory-open attempts. Snapshot summaries expose these bounded rows along with coordinator aggregation time and retained-handle limit/peak. The interface provides an expandable table, entry percentages excluding the root, and JSON copying without file paths. Settings shows optional P/E hardware counts while explicitly leaving scheduling to macOS.

- **25 core tests passed**, including ordered idle/precancelled/skewed worker rows, exact root-excluded entries/issue sums, descriptor-attempt accounting, read-only native clock/QoS getters and camelCase serialization. The initial test-only QoS prototype had an incorrect argument list and was corrected to the installed SDK's three-argument signature before any app or real scan was run. Native CPU/QoS and descriptor telemetry received independent review; final disposition approved with no blocking findings.
- **21 shell tests passed**, two unrelated opt-in tests ignored. Added summary-field serialization and exact worker accounting integration.
- **37 frontend tests passed**, including percentages, pending/unavailable CPU readings, per-worker time distinctions, retained-handle labels and copied report semantics/failure handling. The added descriptor label/copy assertions passed in a focused rerun.
- **15 benchmark-tool tests passed**, including absent CPU clocks, zero elapsed time, missing descriptor fields, worker IDs, and root/issue accounting drift.
- Formatting, strict Clippy, release CLI, production frontend and host-architecture `.app` bundle builds passed.
- Real release CLI diagnostics captured startup scans at 5/7/14 workers, then 7/14 with descriptor counters. All worker accounting matched their snapshots. Baseline/candidate overhead was checked in two alternating seven-worker pairs. [Raw measurements and limitations](BENCHMARKS.md) distinguish worker wall phases from CPU utilization and P/E placement.
- Browser visual check expanded the real 14-worker CLI data table and showed percentages/timing/open-attempt/handle-limit labels correctly. It performed no filesystem actions. Clipboard serialization/error handling was verified by frontend tests; browser/native clipboard availability remains platform dependent.

The user's completed app session was left intact; the rebuilt bundle is ready for relaunch. Per-core placement and end-to-end GUI overhead were not measured, and no scheduling/affinity policy was changed. Original full-scan performance acceptance remains outstanding.


## QoS Settings follow-up

Added macOS scheduling preferences Automatic / Prioritize scan / Background alongside worker count. Only newly spawned scan worker threads request QoS; Automatic makes no setter call. Worker count and policy are captured together in each session, so saving cannot alter an existing scan. Legacy preferences migrate to Automatic. Explicit policy failures appear as `qosError` independently of filesystem issue totals; unsupported platforms offer only Automatic. Existing IPC capabilities and cleanup/save serialization are unchanged.

- Workspace/all-target tests: **52 passed** (27 scanner, 2 CLI argument tests, 23 native shell); two unrelated opt-in tests remain ignored.
- Frontend tests: **40 passed**, including QoS-only changes, both-policy default reset, disabled cleanup/unsupported controls, current-session versus next-session policy, and separate native-error presentation/copy-report policy.
- Python tooling tests: **21 passed**, including six QoS harness checks for rotation, inherited Automatic class, missing/failed/wrong QoS, executable mutation, initial/warm separation and aggregate drift. The later mode-subset option passed focused rotation/summary checks and Main review; it preserves the default three-mode run.
- Formatting, strict workspace Clippy, frontend production build, release CLI and local host `.app` build passed.
- Native setter tests use disposable spawned threads for user-initiated and background QoS. Installed Xcode SDK `pthread/qos.h` verifies the two-argument setter ABI, class values, zero relative priority, and returned error codes. Context7 resolved the Apple Kernel docs but had no matching setter documentation, so the official SDK was used. Non-macOS behavior was reviewed, not executed.
- Independent reviewer_high review approved the stable worker/native/UI/tooling candidate with no blocking findings. Prior native parsing, traversal, authority and Trash reviews remain applicable; this change does not alter those operations.
- Browser-only visual fixture displayed the actual Settings component with all three modes, saved a QoS-only change, and showed next-scan feedback while the current policy stayed Automatic. Native commands were simulated only in this fixture; it changed no real preference. Screenshot showed the selector and feedback without clipping. Temporary tab and dev server were closed.
- The rebuilt bundle is at `target/release/bundle/macos/Hahakigami.app`. The user's existing process and in-memory results were preserved; the new bundle was not relaunched over that session. Full Disk Access setup remains manual, and a rebuilt executable may require its setup acknowledgement again.

QoS timing reports and measurement limits are recorded in BENCHMARKS.md after the exploratory comparisons. They do not establish actual P/E-core placement or satisfy the outstanding OpenDisk app-level acceptance gates.


The completed QoS comparison ran 20 fresh release CLI startup scans, with one initial and three warm samples per selected policy/count. All runs completed and passed requested/observed QoS, binary-stability and worker accounting checks. Warm medians: 7 workers Automatic **17.553 s**, Prioritize scan **17.568 s**, Background **79.849 s**; 14 workers Automatic **25.718 s**, Prioritize scan **26.218 s**. Aggregate coverage drifted, especially during Background scans, so these remain exploratory measurements. No speed improvement was demonstrated; Automatic stays the default. Raw records are `benchmark-results-qos-settings-7.json` and `benchmark-results-qos-settings-14.json`; BENCHMARKS.md records initial/warm samples, RSS and coverage limits. Final tooling suite passed all **21 tests**. No benchmark or dev-server process remained running.


## Automatic workers follow-up

Added Auto/manual worker mode with a conservative fresh default of `min(4, available CPU threads)` and a shared maximum of 16. Old numeric preferences migrate to Manual. Successful calibration choices persist in the same atomic preferences record, matched by stable filesystem UUID, Mac model/architecture/core counts, QoS and profile format version. Unknown, mismatched, malformed or unsupported volume profiles use the conservative count. Failed persistence leaves the previous preference/profile unchanged; the file is bounded at 64 KiB and sixteen profiles.

Calibration requires a current completed scan and saved Auto settings. It preserves those results, opens the authoritative root before handoff, and runs off the UI thread. A single native job gate excludes new scans, preference writes, cleanup and setup/restart; the UI also freezes target changes. Running/cancelled/failed/completed messages carry source scan and calibration IDs. Cancellation and profile commit are serialized; stale or early terminal messages cannot regress the UI. A new job revokes abandoned cleanup tickets, and successful cleanup retains its required full rescan behavior.

The scanner chooses stratified directory jobs with required ancestors from the completed tree, bounded to 4,096 directories and 250,000 nodes. Two fresh descriptor-anchored preparation scans, separated by a cancellable one-second observation interval, refresh stale membership and prune unstable jobs with their dependent descendants. Root-listing drift, metadata failures, insufficient samples or later membership changes fail safely. Timed trials use the fixed remaining directory set through the production scanner; full-scan traversal defaults stay unchanged. File contents are not read; mutable reported sizes do not affect membership fingerprints.

Every available worker count is measured once for warm-up and three times in rotated warm rounds. Each timed trial must be at least 50 ms; incomplete measurements fail, noisy candidates are excluded, and the lowest count within 5% of the fastest stable median is selected. The shared 90-second budget includes preparation and is cooperative; a blocked OS call can delay stopping. Profiles estimate concurrency for a sample and do not guarantee the fastest full scan or P/E placement.

- Final Rust checks: **74 passed** (38 scanner, 2 CLI, 34 native); three opt-in checks ignored in the ordinary suite (including the separately run calibration probe).
- Frontend: **46 passed**, covering Auto/manual selection and legacy migration, target/profile refresh, fixed active-session workers, calibration eligibility and cancellation across views, stale/early terminal events, and conflicting scan/save/cleanup/target actions.
- Tooling: **23 Python tests passed**, including the native-probe report validation. Formatting, strict workspace Clippy and frontend builds passed.
- New native tests exercise UUID parsing and held-root stability, volume/hardware/QoS matching, bounded profiles and rollback, native operation gates, stale/running/tiny-source rejection, preservation of completed results, and calibrated counts captured by new sessions.
- Independent reviewer_high approved the engine, UUID ABI, authority, job locking, profile persistence and UI. Focused follow-ups approved bounded refresh/pruning, the shared deadline and the cancellable observation interval; unaffected native cleanup and IPC evidence was reused.
- Actual App/Settings browser fixture showed Auto, conservative choice, calibration eligibility, running progress and Cancel in both views, blocked scan/cleanup/settings/folder controls, and recovery after cancellation. Simulated commands changed no real filesystem data or preferences. The temporary tab and dev server were closed. A later small hint/target guard correction passed the same 46-test suite and rebuilt frontend.
- User preferences and the open native app session were preserved; `.app` builds target the host architecture. Full Disk Access remains manual. Native command probes use only disposable preferences and inherited permissions; they are not `.app` UI or OpenDisk acceptance comparisons.

Initial implementation probes are retained as `benchmark-results-auto-calibration-*-initial.json`: GitHub succeeded and used its saved five-worker choice on the next full scan; startup correctly declined stale membership and retained four. Those observations prompted the preparation step. Final-candidate probe results are recorded in BENCHMARKS.md, separately from these initial observations.


Final-candidate release probes passed their integration checks on GitHub and startup with unchanged binary hashes. GitHub completed 56 sample trials, selected/saved/reloaded **five workers**, and the next full scan used five with matching 698,640 entries and zero issues. Startup preserved its completed 4,121,816-entry result and declined changing membership after 43/56 trials; Auto retained four and saved no profile. No full-scan optimum or speedup is claimed from these ordered observations. The final host `.app` build passed; it was not relaunched over the user's in-memory session. No calibration or development-server process remains active.


## Calibration failure recovery (11 October 2026)

A user screenshot showed a startup calibration stopping at 12/56 trials with 13 workers and an ambiguous metadata/budget error. Read-only inspection confirmed the app retained its completed 4,170,921-entry, four-worker result. The old build discarded the underlying sample failure, so its exact cause cannot be recovered from that banner.

The scanner now retains I/O kinds and native error codes in optional, backward-compatible ScanIssue fields. Calibration distinguishes changed membership/identity, missing or inaccessible folders, listing growth, parser faults, resource failures, root changes, cancellation and global node limits. Error details include the first affected folder/code/count when available, rather than combining unrelated failure classes.

Transient failures can prune affected sample jobs and every dependent selected descendant. Recovery validates captured root/tree/scope/bounds and never adds directory identities or accepts the failed timing. Every retry restabilizes its new fixed sample and discards prior scores; selection uses only the final complete attempt. There are at most three attempts under the same 90-second cooperative deadline. Confirmed non-root EMFILE/ENFILE failures can lower the tested ceiling and restart; other resources, parser errors, unknown I/O, root failures and global limits stop without a profile commit. Progress is monotonic across attempts, includes the attempt number, wraps long paths, and retains cancellation/conflict controls.

- Corrected workspace/all-target suite: **90 Rust tests passed** (52 core, 2 CLI, 36 native); three opt-in tests remain ignored in the ordinary run.
- **46 frontend tests passed**, including retry progress followed by terminal completion without stale-message regression; TypeScript/Vite build passed.
- **24 Python tooling tests passed**, including rejection of final-attempt scores supplemented by older attempts; formatting and strict workspace Clippy passed.
- Independent reviewer_high found R7: first-arriving EMFILE/ENFILE could mask another fatal issue. The correction defers only eligible descriptor pressure until all issues and root membership are examined; parser, unknown-I/O, root and ENOMEM failures take precedence in either order. Both-order mixed-fault tests and resource-only/root-drift checks passed. The reviewer approved the corrected candidate and reused unaffected authority, pruning, locking and cleanup evidence.
- Browser-only fixture verified the detailed retry banner, wrapping, attempt counter, disabled conflicting controls and cancellation. It changed no real scan or preference. The temporary tab and server were closed.
- Corrected release test executable and host `.app` built successfully. The user's active process, completed results, settings and macOS grants were preserved; the new bundle was not relaunched over their session.

Final release-probe evidence and remaining limits are recorded in BENCHMARKS.md. Native command probes use disposable preferences; they do not identify the exact discarded error in the earlier user session or establish formal OpenDisk acceptance.


The first corrected release probe retained a precise failure at 13 workers: `Interrupted system call` (`EINTR`, OS error 4) while enumerating `~/Library/Containers/com.apple.FinalCut/Data`. The old screenshot does not establish the same cause, but this reproducer exposed missing interrupted-call handling. This report is preserved as `benchmark-results-calibration-recovery-startup.json`; its failed timings are not used for worker selection. The companion project probe completed and used its saved four-worker choice. A subsequent affected-scope fix adds bounded retries of interrupted metadata operations and restarts an interrupted directory listing through a fresh identity-validated held-descriptor cursor, dropping partial listing data. Closing descriptors is never retried.


The EINTR correction also passed an independent reviewer_high affected-scope review. R8 identified that a terminal interruption could discard earlier per-entry failures; both bulk and portable enumeration now retain those failures alongside terminal errors. Permanent issues prevent retries from making a partial pass appear clean, while clean interrupted listings retry with a fresh anchored cursor. Regression checks cover prior EACCES, ENOENT and EMFILE followed by EINTR, mixed terminal I/O, listing limits, cancellation, retry exhaustion and independent descriptor position. R7 and R8 are closed; prior authority and cleanup evidence was reused.


Final corrected release probes passed on startup and GitHub using one unchanged hashed executable. Both completed 56 trials on attempt one, selected/saved/reloaded five workers and used five on the following full scan. Startup retained its completed source tree and reported 449 inaccessible/error entries in both full scans; GitHub had matching 705,299 entries and zero issues. Detailed records and timing limits are in BENCHMARKS.md. The final TypeScript/Vite production build, release native test executable and host `.app` build passed. The user’s live app was preserved, and no native probe or development server remains running. Reopening the rebuilt app and any Full Disk Access acknowledgement remain user actions; no grant was changed.

## Treemap size labels (11 October 2026)

Roomy terminal treemap blocks display the allocated size beneath the existing name in bold 16 px text, with one decimal for scaled units (for example, 56.0 GB). Labels require sufficient height and measured text width; small blocks and expanded parent rectangles do not gain overlapping labels. Existing formatting outside the map keeps its prior precision.

All **47 frontend tests passed**, including bold label drawing, short/narrow block suppression, expanded-parent suppression and sunburst behavior. TypeScript/Vite production build passed. A browser-only fixture using the real DiskMap component showed 56.0 GB, with fitting labels and uncluttered small cells. Synthetic data involved no native commands or user preferences; the temporary tab and dev server were closed. The host .app was rebuilt without relaunching over the user's existing scan.

## Hardware-based Auto workers (11 October 2026)

Replaced calibration-driven Auto with OpenDisk-style scope heuristics and Radix-style power/thermal reductions. Normal counts are half the active CPUs clamped to 4–8 for volume roots, or a quarter clamped to 3–5 for folders; actual CPU and scanner limits also apply. Low Power Mode halves the count, then fair temperature subtracts one or serious/critical temperature halves it, always retaining one. Unknown conditions are neutral. Native NSProcessInfo getters are read-only; scope classification uses fstatfs on the authorized held root rather than a new path lookup. Conditions are queried again for each new scan and the chosen count stays fixed for that session.

Saved Auto settings now ignore historical calibration profiles. Legacy numeric/manual settings retain their count, and Manual bypasses power/thermal reductions. QoS choices remain unchanged. Settings shows the selected scope, base/effective count and current conditions. Calibration controls, frontend lifecycle and their three capability grants are removed; the restricted AppManifest still lists those retained internal commands, so Tauri denies frontend invocation. Internal sample-engine tests and historical reports remain available, but the current Auto integration probe is now scripts/workers_probe.py; calibration_probe.py identifies its retired status.

- **93 Rust tests passed** (52 scanner, 2 CLI, 39 native); three opt-in checks ignored in the ordinary suite.
- **45 frontend tests passed**, including hardware Auto hints, power/thermal copy, target refresh/stale-result rejection, numeric/manual migration, saving Auto, active-session preservation, and absence of calibration IPC.
- **25 Python tooling tests passed**, including worker-scope caps, immutable captured counts, retired calibrated sources and persisted Manual checks.
- Formatting, strict workspace Clippy and TypeScript/Vite production build passed.
- Independent reviewer_high approved count bounds, migration, session capture, held-descriptor classification, read-only Foundation getters and actual Tauri ACL denial of ungranted retained commands. Prior scanner parsing, cleanup and traversal approvals were reused.
- Browser-only fixture using the real Settings component showed seven workers for a volume and three for a folder on a modeled 14-thread Mac, with conditions and active-scan hints fitting. It used simulated commands only. The temporary tab and development server were closed; user preferences, grants and the live app remained intact.

Host release test/app builds and read-only native-command integration observations are recorded in BENCHMARKS.md. This heuristic does not establish the fastest worker count, P/E placement or formal OpenDisk performance acceptance. No continuous thermal observer or resizing of active scans is introduced.

Final hardware Auto release probes passed on startup and GitHub with the same unchanged executable hash: seven workers for the volume and three for the folder, both at nominal temperature with Low Power Mode off. They preserved captured counts and verified disposable Manual persistence/reload. Source entry/error totals and measurement limits are recorded in BENCHMARKS.md. The host release `.app` build passed; it was not relaunched over the user's existing in-memory scan. No development server or probe remains running.

## Settings removal (11 October 2026)

The app now opens directly to the explorer, with no app Settings tab, view switching, worker selector, QoS selector or settings-save IPC. Full Disk Access setup, scan cancellation/progress, diagnostics, map/tree navigation, Trash confirmation and required post-cleanup rescan remain connected. Unused Settings components, tests, styles and DTOs were removed.

Every new production scan now forces hardware Auto and Automatic scheduling, even if an older preference file contains Manual workers or Background/UserInitiated QoS. The file is read for legacy internal helpers but never rewritten by the new scan flow. Old counts cannot remain silently active after their controls are gone. Scope/power/thermal formulas are unchanged, and active sessions keep their captured workers. The main capability grants for get_scan_settings and set_scan_workers were retired; both commands remain in the restricted AppManifest and are denied to the frontend. Historical internal settings/sample helpers are retained only for research and regression coverage.

- **94 Rust tests passed** (52 core, 2 CLI, 40 native); three opt-in checks ignored in ordinary runs.
- **34 frontend tests passed**, including no app Settings/IPC, retained diagnostics, Full Disk Access setup and cleanup confirmation/rescan coverage.
- **25 tooling tests passed**, including rejection of a legacy preference overriding production Auto.
- Formatting, strict workspace Clippy and TypeScript/Vite build passed.
- Independent reviewer_high approved the forced production policy, permission retirement and preserved setup/cleanup flow, reusing unchanged hardware-policy, native-authority and Trash evidence.
- Actual App component in a synthetic browser fixture opened directly to the explorer and displayed completed progress, seven workers, diagnostics and the bold 56.0 GB map label. No Settings control appeared. No native commands, real scan, cleanup or preference write occurred in this fixture. The temporary tab and dev server were closed.

The live user app and its in-memory results remain untouched. The built bundle must be reopened to use the updated interface; Full Disk Access grants remain manual. Release probe records are linked from BENCHMARKS.md and do not establish original performance acceptance.

Final host `.app` bundling passed. A corrected release native-command probe completed the GitHub scan with three Auto workers and Automatic scheduling, and verified that a reloaded disposable legacy Manual choice could not override the production policy. Executable hashes were unchanged throughout. Hardware-policy and startup evidence from the preceding phase was reused; no full-disk performance comparison was repeated. No probe or development server remains running.

## Full Disk Access popup recovery and local signing (11 October 2026)

A user still received a macOS Documents access prompt after acknowledging Full Disk Access. Read-only signature inspection found the delivered bundle had only the linker ad-hoc signature: a hash-suffixed executable identifier, a designated requirement pinned to that build's cdhash, unbound Info.plist and no sealed resources. This confirmed an unstable application identity, a known source of TCC grant confusion. It does not independently prove the state of the user's OS grant.

The new `pnpm desktop:build` helper selects one existing valid Apple Development identity, or fails for missing/ambiguous selection. Explicit `--identity` must match a valid local development identity; `--adhoc` is an explicit unstable fallback. The helper removes Apple credential/import/notarization environment variables, passes the selected identity via structured Tauri arguments/environment, and verifies the final app with strict/deep codesign validation plus the configured bundle identifier, development authority/team, sealed resources and a designated requirement without cdhash. It creates/imports no certificate, exports no key, changes no keychain trust/ACL, and grants no privacy permission. README and AGENTS.md use this build path to avoid future unsigned bundle regressions.

The FDA setup modal now explains recovery: quit, remove the old Full Disk Access entry, re-add the exact app displayed, enable it and reopen. Checkbox confirmation remains explicitly an acknowledgement, not OS permission verification. No private TCC API or protected-folder/database probe was added. Apple DTS recommends stable development signing for TCC and advises against treating the TCC database's filesystem location as a public permission API.

- **34 frontend tests passed** with exact-path recovery copy and the existing checkbox/scan/picker/restart gates; TypeScript/Vite build passed.
- **28 Python tooling tests passed**, including fail-closed identity selection, credential filtering, sealing/authority/team/identifier requirements and actual designated-requirement output forms.
- Independent reviewer_high found R9: codesign can emit `designated =>` without `#`. The corrected parser accepts both formats while preserving rejection of missing/cdhash requirements; R9 was closed and the workflow approved. Unaffected native setup, traversal, cleanup and policy evidence was reused (94 prior Rust passes; Rust source unchanged this turn).
- The corrected full `desktop:build` workflow passed and produced a strictly verified, sealed Apple Development-signed host `.app` with identifier `app.hahakigami.desktop` and a stable certificate-based requirement.
- Two disposable fixture executables had different signed hashes but identical designated requirements under the same existing development identity. Both signatures verified; neither program was run. The proof report omits certificate holder details: [signing-verification.json](signing-verification.json).
- Native UI inventory confirmed Hahakigami was not running before verification. The new signed bundle launched successfully and displayed the updated Full Disk Access setup, exact bundle path and disabled save/restart until manual checkbox confirmation. No folder scan, OS grant or checkbox action was performed. The app was left at setup for the user's manual reauthorization.

The user must re-add the new signed app to Full Disk Access and complete setup. Effective FDA and absence of all permission prompts have not been tested after reauthorization. A stable signature fixes rebuild identity churn; it does not override macOS privacy decisions or Data Vault restrictions. Setup acknowledgements still match the executable fingerprint and require reconfirmation/restart after rebuilds, even when macOS retains the certificate-based grant. Distribution signing/notarization and original performance acceptance remain deferred/outstanding.

Sources: [Apple DTS stable development-signing guidance](https://developer.apple.com/forums/thread/663889), [Apple staff on ad-hoc identity changes](https://developer.apple.com/forums/thread/819406), [public FDA API limitations](https://developer.apple.com/forums/thread/114452). Context7 Tauri signing documentation and local CLI/bundler source were checked for signingIdentity/APPLE_SIGNING_IDENTITY and credential handling.

## Live Full Disk Access entry repair and scan (11 October 2026)

After the user's manual setup confirmation, the signed app's startup scan stalled at 1,911,650 entries (last progress time 7.5 seconds). A one-second sample of the owned app showed all seven scan workers waiting in openat, not a Rust mutex. App-specific unified logs reported a denied Full Disk Access preflight (authValue=0) because the saved requirement used an old cdhash, followed by a pending SystemPolicyAppData permission request. The user confirmed a new permission dialog. System Settings showed Hahakigami switched on, and Show in Finder revealed the correct current bundle; the visible switch alone did not mean its cached grant matched the current signature.

The user explicitly authorized repairing only Hahakigami's entry. Cancellation was requested, but the OS-held calls could not return until the request ended. Computer use refused direct access to UserNotificationCenter for safety; quitting the requesting Hahakigami app withdrew the pending request without granting additional app-data access. System Settings was closed and reopened to refresh app metadata. Only Hahakigami's stale entry was removed, then the exact signed bundle was selected and added again with Full Disk Access enabled. Other displayed app permission states remained unchanged. No tccutil reset, TCC database access, private API, daemon reset, signature weakening or app rebuild was used.

The same signed bundle was reopened and its startup scan completed without new permission dialogs: **4,175,201 entries**, **18.2 seconds**, **seven workers**, **670.8 GB displayed allocated**, and **278 skipped/unreadable issues**. The result remains marked incomplete; the last displayed issue was an intentionally excluded mounted filesystem. No claim is made that every issue is a permission error or that the whole volume is exactly attributed. This is one GUI verification run, not an alternating performance comparison or median. The completed in-memory result was left open for the user.

The attempted/stalled run and repair occurred after the earlier signing verification; the earlier statement that no OS grants were changed applies to signing itself. The entry replacement here was performed only after the user's explicit action-time approval.

## Truthful scan issues and exclusions (11 October 2026)

The former “skipped or unreadable entries” count combined expected scope/mount exclusions with actual metadata/read failures. It did not mean that many unreadable files: one directory failure can affect a whole branch, and multiple events can refer to the same path. The historical 278 count from the preceding live run cannot be retrospectively split using its old frontend/native contract. No replacement real-disk scan was run for this change; that completed in-memory result and OS privacy grants were preserved.

Expected exclusions now have separate counters/details and do not mark an otherwise completed scan incomplete. Actual failures still mark results incomplete and preserve their native reason, error kind and OS error code when available. Every accepted node stores its exclusion status independently of capped detail retention, so excluded folders show “Excluded / Not scanned” and remain protected from cleanup even after 10,000 retained exclusions. Verified firmlink mirrors, traversal authority, mounted-filesystem boundaries, hard-link accounting and scan worker policy are unchanged.

“Show scan details” exposes separate categories with retained paths/reasons in pages of at most 100. Totals count events/locations; details retain up to 10,000 per category and explicitly disclose truncation. Rows have no file actions. The native command accepts only an active scan ID, a typed category, offset and bounded limit; it reads the in-memory snapshot without touching the filesystem. The response identifies its owning scan root, category and requested offset. Its generated permission is granted only to the bundled local main window. Stale/mismatched responses are rejected by the UI. First expansion selects the latest nonempty category, and subsequent explicit tab choices remain selected.

- **99 workspace Rust tests passed** (55 core, 44 native); three existing opt-in checks remained ignored. The scanner worker also passed the two CLI example tests. Tests cover independent counters/retention, native error metadata, backward-compatible serialization, bounded pages and unusual names, stale/invalidated sessions, exact node flags and beyond-retention cleanup rejection.
- **45 frontend tests passed** across eight files, including exclusion-only completion without a failure/FDA warning, issue reasons, pagination/truncation/retry, stale category/scan replies, Running-to-Completed initial category selection, excluded tree/inspector behavior and disabled cleanup.
- Formatting, strict workspace/all-target Clippy, and TypeScript/Vite production build passed. Unchanged tooling/signing tests and previous traversal/cleanup reviews were reused.
- Independent reviewer_high found R10 (node semantics incorrectly depended on capped detail paths) and nonblocking R11 (initial empty running counts selected the wrong details category). Both were corrected in one review round; complete per-node markers and current-count initial selection have regression coverage. The stable corrected candidate was **approved**, with no remaining blockers in the reviewed scope.
- The actual App component in a synthetic browser fixture displayed separate issues/exclusions, readable path/native reasons, the 100-row/one-row next-page boundary, and neutral “Scan complete” with exclusions alone. Selecting an excluded directory showed “Not scanned,” scope guidance and disabled Trash. The fixture issued no real native commands, filesystem reads or privacy changes; its temporary tab and development server were closed.

The updated bundle must be reopened and a fresh scan performed to obtain the new accounting. Existing scan results are in memory and are not migrated. Full Disk Access setup acknowledgement still follows the executable fingerprint; this does not change the macOS grant or add a permission probe. Original full-scan performance acceptance remains outstanding.

Final `pnpm desktop:build` passed, producing the optimized host `.app` with strict/deep verified Apple Development signing and a stable certificate-based designated requirement. The generated ACL manifest confirms get_scan_issues is explicitly permitted only by the local main capability. Native app inventory confirmed the existing Hahakigami process remains running after bundling; it was not restarted or rescanned. No benchmark speed/readiness claim follows from this UI/accounting change.

## Preserve Full Disk Access setup acknowledgement across updates (11 October 2026)

The user reopened a newly signed build and saw our setup modal despite Hahakigami being enabled in the macOS Full Disk Access list. This was the app's own acknowledgement gate, not a new macOS permission request: the preference matched executable size and modification time, so every rebuild discarded the confirmation. Read-only inspection of only Hahakigami's own configuration confirmed an existing legacy acknowledgement for the current delivered executable path.

Setup now remembers an exact executable path in a version-2 acknowledgement. It accepts the complete legacy schema at that same path regardless of rebuilt size/mtime, without rewriting preferences on load. A different executable path, missing/malformed/oversized record, unknown version or mixed schema still requires initial setup. The original initial-confirmation restart gate remains; confirming from a running process whose executable was replaced still fails. No OS permission status is inferred, probed or claimed from the preference, and macOS independently enforces actual grants. The dialog explicitly allows confirming already enabled access and explains persistence across updates. This supersedes earlier notes that setup acknowledgement followed every executable fingerprint.

- **101 workspace Rust tests passed** (55 core, 46 native), with three existing opt-in tests ignored. Five focused setup tests cover current and legacy rebuild persistence, different paths, initial restart, replacement while running, invalid/future/mixed schemas, failed atomic save cleanup and input bounds.
- **45 frontend tests passed**; formatting, strict workspace/all-target Clippy and TypeScript/Vite production build passed.
- Independent reviewer at Medium approved persistence, migration, truthful UI wording and unchanged native gates/capabilities, with no blocking findings.
- Existing application sessions were not restarted. No protected-folder/TCC probes, private APIs, privacy-grant changes, preference writes to the user's setup record, keychain changes or real disk scans were performed.

Reopening the delivered update loads the existing acknowledgement automatically at this path. Removing/re-adding the macOS Full Disk Access entry is unnecessary for this acknowledgement fix. This is not verification of every OS privacy permission or the original performance acceptance.

The final `pnpm desktop:build` succeeded and strictly verified the sealed Apple Development-signed host `.app` with the existing stable designated requirement. The current app was left open; the updated bundle was not launched over it.

## Preserve child paths in metadata failures (11 October 2026)

A screenshot showed 269 scan issue events and nine expected exclusions, including three identical `/Library/Caches` EPERM rows. Source inspection found that successful directory entries carried their child paths but failed child metadata lookups returned only an `io::Error`. The scanner consequently recorded these failures against the parent. The screenshot alone cannot establish whether those particular old rows were distinct child failures or directory failures; the discarded names cannot be reconstructed from the old snapshot.

Both Darwin bulk enumeration's directory/symlink metadata refinement and portable descriptor enumeration now attach the known child path to failed entry lookups. The scanner unwraps that context while preserving the original error message, kind and native OS code. Terminal directory-read/open failures still report the directory. Interrupted listing conversion reads the original native cause, retaining EINTR and existing bounded retry behavior. Permanent failures continue to prevent retry concealment. Successful entries do not allocate an error context or perform additional filesystem calls. No parser, traversal, identity validation, mount policy, counters, retention limits or action authority was changed; failures are not suppressed or deduplicated.

The scan-details warning now explains that file permissions and other macOS protections can still deny access with Full Disk Access enabled, and the error alone does not identify the restriction. No specific displayed path is labeled a Data Vault or diagnosed as a missing FDA grant. [Apple DTS guidance](https://developer.apple.com/forums/thread/124895) describes BSD permissions, data vaults and FDA as separate permission mechanisms.

- **104 workspace Rust tests passed** (58 core, 46 native), with three existing opt-in tests ignored. New regressions cover child paths and native EPERM/EACCES/ENOENT causes through completed listings, directory EIO attribution, EINTR code preservation, actual descriptor-relative ENOENT with quote/newline/Unicode filename, and lazy success context. Existing interruption, traversal, hard-link, native parsing and retention tests passed.
- **45 frontend tests passed**, including the qualified permission explanation; formatting and strict workspace/all-target Clippy passed.
- Independent reviewer at Medium approved path/cause propagation, retry invariants, unchanged authority and qualified UI wording, with no blocking findings.
- No real disk scan, protected-folder/TCC probe, privacy grant change, live app restart or new privilege was performed. Existing snapshot rows are not rewritten; reopening and rescanning is required for precise child paths. Original performance acceptance remains outstanding.

Final TypeScript/Vite build and `pnpm desktop:build` passed. The optimized host `.app` was sealed and strictly verified under the existing Apple Development identity and stable designated requirement; the user's running app was not relaunched.

## Copy scan errors to clipboard (11 October 2026)

A “Copy errors” button beside the scan-details toggle collects every retained error page, regardless of which detail category is selected. It writes formatted JSON with scan/root identity and state, event paths/messages/native error fields, total/copied/omitted counts and truncation/partial-result notes. Expected exclusions are not included. The initial retained prefix is captured independently of later events appended during a running scan. Requests stay bounded to 100 entries per page and at most 10,000 retained events. Repeated events and quote/newline/Unicode paths are preserved by JSON serialization.

Clipboard text writing uses the official Tauri clipboard-manager plugin, pinned to 2.4.1 in both dependency sets and lockfiles. Current Context7 documentation and the installed plugin permission/command source were checked. Only `clipboard-manager:allow-write-text` is granted to the local main window; clipboard reads, default clipboard grants and remote access are not enabled. Existing filesystem/query/action authority is unchanged. The package manager's temporary fresh-release exception was removed by selecting the older stable plugin release; existing supply-chain policy settings remain unchanged.

The button is disabled while copying and reports success or clipboard failure. A page failure, invalid scan/category/root/offset, truncated page or stale pre-write request prevents copying an incomplete report. New scan/unmount cleanup invalidates old requests. Native clipboard writing happens once, after collection succeeds. An already dispatched native clipboard write cannot be cancelled; a subsequent scan prevents stale feedback but cannot undo that explicitly requested write.

- **50 frontend tests passed** across eight files, including multi-page/category-independent copying, repeated events and unusual names, truncation and cancelled state, later-page failure, stale scan replies/unmount, disabled pending controls and native clipboard failure/retry.
- **104 workspace Rust tests passed** (58 core, 46 native), with three existing opt-in checks ignored. Formatting, strict workspace/all-target Clippy and TypeScript/Vite production build passed.
- The generated ACL manifest confirms the clipboard permission allows only `write_text`; only the local `main` capability grants it. Independent reviewer at Medium approved export bounds/staleness and the new native permission, with no blocking findings.
- Final `pnpm desktop:build` succeeded and strictly verified the sealed Apple Development-signed host `.app` with the existing stable designated requirement. Existing app sessions, scans, Full Disk Access grants and the user's clipboard were left untouched. Tests mock the clipboard operation; no claim is made of a live OS clipboard round-trip in this verification.

Reopen the delivered app and perform a fresh scan to use the new button. Existing in-memory results are not migrated between app processes. Original full-scan performance acceptance remains outstanding.

## Radix-inspired system restriction presentation (11 October 2026)

The user approved quieter handling after comparing upstream behavior. Radix retains permission warnings/inaccessible nodes and can present recognized system protections separately from other failures. Hahakigami independently implements that presentation idea; no upstream source was copied. Reference: [Radix warning presentation](https://github.com/colinvkim/Radix/blob/df1235bfb05acb7a678718166491bcc27817d95d/Radix/Shared/ScanWarningsPopover.swift).

A pure macOS classifier requires both native PermissionDenied kind and EPERM/EACCES, together with a conservatively recognized system path. It rejects relative, dot/dot-dot, doubled-separator, NUL and lookalike paths. System/Data aliases share the same rules. Home, ordinary application locations, /usr/local, user temp projects and ambiguous paths remain in the prominent other-issues category. Internal /var/folders rules require a validated daemon bucket or a direct Apple-named C/0 namespace; the dmd exception is restricted to a direct 0/dmd component. Scheduled-job and mail-spool rules cover specific known locations rather than all /var/lib. The classifier uses stored native metadata only: it does not identify the actual OS protection or inspect FDA status, ownership, TCC or protected directories.

ScanStats counts every restricted-system event before applying the 10,000-detail retention limit, while preserving the original all-error count, incomplete flag, original records and scan coverage. No directory is newly pre-excluded and no warning is discarded. Native summary exposes the restricted count and only promotes the first retained non-restricted failure. Grouped read-only detail queries accept restricted/other as additive categories with filtered retained counts and full category totals; the existing errors category remains ALL failures for Copy errors. Pages remain bounded to 100, and authority/stale scan handling is unchanged. Copied records include additive category metadata.

The interface shows a neutral “Restricted system locations” notice and quiet completion heading for system-only restrictions, explicitly explaining unavailable data and incomplete totals. Other failures remain prominent. Details separate other issues, system restrictions and intentional exclusions. Copy errors retains every stored failure across all categories. Existing node incompleteness, cleanup protection, worker diagnostics and reported allocated accounting remain unchanged.

- **113 workspace Rust tests passed** (65 core, 48 native), with three existing opt-in checks ignored. Two CLI checks also passed during scanner development. Tests cover conservative path/kind/code matching, aliases/lookalikes/user locations, full counters past retention, serde defaults, filtered pagination, full vs retained category totals, ALL-error export compatibility, prominent unknown/private failures and restriction-only incomplete summaries.
- **53 frontend tests passed** across eight files; formatting, strict workspace/all-target Clippy and TypeScript/Vite production build passed.
- The actual Rust classifier categorized all **269 events** in the user-supplied report as system restrictions, with zero other issues. Only aggregate results were recorded; private paths/hashes and the pasted report were not copied into source or public documentation. This classification is not an FDA permission check or proof of a specific OS protection.
- Independent reviewer at Medium approved classification, retention/full-count accounting, grouped query bounds, unchanged authority and truthful UI behavior with no blocking findings.
- The actual App in a synthetic browser fixture displayed the neutral completion/header, incomplete-coverage guidance, available Copy errors control and restricted/other/excluded detail tabs with pagination. No real native commands or clipboard writes were executed. The temporary tab and development server were closed.
- Final `pnpm desktop:build` passed, producing the strictly verified sealed Apple Development-signed host `.app` under the existing stable designated requirement. The user's live app was not restarted, scanned or granted new permissions; OS access and clipboard were untouched.

Reopen and rescan to use the new presentation and counters. Original scan coverage and full-scan performance acceptance are unchanged; no new speed or complete-attribution claim is made.

## Conservative incremental rescans (11 October 2026)

Milestone 2 adds a private optimization cache separate from the scan results, public FSEvents history replay, and move-based clean-listing reuse. Boot/journal UUIDs, fresh root identity and an original full-scan age limit are required. Local clean folders without hard links/exclusions can seed reuse; startup disks, restricted/incomplete scopes, remote filesystems and scopes containing the cache remain full scans. Shared-allocation or uncertain history events trigger full traversal. Every directory still opens through descriptor identity validation. The standalone scanner transfers its final owned snapshot rather than cloning the entire tree; no numeric peak-memory reduction is claimed.

Automatic cache storage retains the latest entry per location within eight-location and 512 MiB compressed-archive limits. UUID-owned interrupted files are cleaned within this cache only. Actual archive sizes drive pruning. Failed writes preserve results and the previous cache. Session completion remains gated through cache finalization, with checking/scanning/caching phases shown in the interface and caching cancellation disabled. Native elapsed time includes this pipeline overhead.

Incremental attempts and hard-link retries retain their trees privately until one final or cancelled publication. Root initialization retries on state/publication phase changes, and issue indexing checks retained records as well as total counts. Reuse never authorizes old paths independently of fresh root/directory guards, and historical cleanup restrictions remain in force.

Event replay honors ordinary FullHistory overlap and dropped/coalesced/wrapped/unknown notices regardless of cursor ordering. The only ignored diagnostic is an old pure mount/unmount record proven unrelated to the ASCII-normalized selected root and ancestors. Public System/Data and private-directory aliases map event paths back into the authoritative namespace; possible case ambiguity and ancestor moves force full scans. HistoryDone and a bounded FlushAsync queue marker are both drained before stop/invalidate/dispatch teardown. The borrowed CFString owner survives through stream release. No TCC/private permission API, protected-folder probe, grant change, credential read or continuous background watcher was added.

- **145 workspace Rust tests passed** (76 core, 69 native), with nine opt-in checks ignored by the ordinary run. New regressions cover fixture parity, root/child identity changes, dirty/nested paths, new/old hard links, symlinks, sparse/size changes, cancellation, hidden retry topology, one-time publication, clock rollback/expiry, separate cache persistence/pruning/corruption, failed cache update preservation, interrupted debris cleanup, overlap/dropped events, alias/ancestor handling, completed-action gates during finalization and issue reindexing after placeholders.
- **66 frontend tests passed** across eight files; TypeScript/Vite production build, workspace formatting and strict all-target Clippy passed. Regressions cover checking/reuse/fallback presentation, cache finalization controls and a same-count placeholder root lookup followed by final publication.
- Three opt-in native journal fixtures passed on disposable local directories: ordinary changes, outside-root hard-link modification, and ancestor rename-away/change/rename-back. Each accepts a conservatively required full scan rather than permitting empty successful replay to hide the change.
- The final native command pipeline parity fixture passed, including persistent cache reload in a new backend state and equality with fresh full-scan path/identity/own-size/aggregate-size signatures. Its final debug observations were **80 ms first full scan**, **189 ms second full fallback**, and **212 ms persisted-cache reload/full fallback**. A global clone-history event correctly required full traversal. These single observations are not release medians or evidence of faster end-to-end rescans. Earlier broader-filter trial timings are superseded and are not performance acceptance evidence.
- The unchanged final core release fixture (20,027 entries) observed **7.7025 ms full**, **3.357125 ms incremental**, **20,004 reused metadata entries**, and **one enumerated directory**. This is one warm fixture pair using supplied dirty paths; it excludes event replay, cache serialization/loading and UI. The earlier debug pair was slower incrementally. No general app-level speedup or original OpenDisk acceptance claim is made.
- Independent reviewer_high at High approved after two correction rounds. Findings covered JSX grouping, topology/ID publication, clock-age checks, ancestor events, interrupted cache debris, and FullHistory continuity; all are closed with targeted regressions.
- The actual App rendered in a synthetic browser fixture with reuse counts and both map controls. No page errors or real native/filesystem/clipboard/privacy operations occurred. The browser and development server were closed.

The packaged application is not launched for these checks. The user's live process and results remain intact. General end-to-end performance and peak-memory comparison remain unverified; conservative global-history fallbacks can be frequent.

Final `pnpm desktop:build` passed after the final presentation guard (covered by the 66-test frontend suite). The helper verified the Apple Development-signed host bundle and its stable designated requirement. Existing app sessions, in-memory results and privacy grants were preserved. Native source remained unchanged after high-effort approval; public notarization remains deferred.


## File category and size filters (11 October 2026)

Added the bundled-main-window-only `filter_nodes` command with validated category/metric and inclusive optional u64 size bounds. Query work runs in a blocking task against the existing session snapshot, with validity checked before and after querying. Matching combines case-insensitive name search, native ASCII extension categories and size predicates across the entire scan, excluding roots and excluded placeholders. Applications include `.app` directories and regular `.pkg`/`.dmg` files; ordinary folders, symlinks and special entries match only All entries. No content inspection, filesystem traversal or archive-format change is introduced.

Results retain largest-first ordering with raw native path and ID ties, bounded pages of at most 500, and one completed-session/query ID-list cache. Pagination returns only its ID slice; neither tree nor entire match list is cloned for transport. Running snapshots clear cache eligibility and recompute; newly opened/replaced sessions get independent caches. The frontend provides category/preset/custom controls, explicit custom Apply validation, a 250 ms cleaned-up search debounce and request guards that reject older results immediately after user input changes. User filter/metric changes clear selections/pages; background live revisions retain valid rows and scrolling while refreshed results load. Filtered results have no inline expansion, but opening a directory clears filters/search and restores browsing. Maps, totals, historical labels and issue details retain their existing semantics.

The final native million-entry release fixture passed once: first filtering/sort **26.044 ms**, cached page **15 µs**, and peak process RSS **216,907,776 bytes** on macOS. This disposable synthetic in-memory measurement includes fixture construction in process peak RSS but excludes webview/rendering and any archive loading; it is not a full-app latency guarantee. Source inspection confirms no full-snapshot clone or full match-list transport.

Final checks passed: **164 workspace Rust tests** (76 core, 88 native; thirteen opt-in checks ignored), **87 frontend tests** across eleven files, TypeScript/Vite production build, workspace Rust formatting, changed-frontend Prettier checks, diff whitespace checks and strict all-target Clippy. New native command tests verify stale/invalid sessions, invalid query arguments, bounded pagination and running updates. Engine fixtures cover every listed extension, native non-UTF-8/case handling, kind boundaries, root/exclusion omission, folder sizes, sparse/hard-link accounting, inclusive/zero bounds, combined predicates, deterministic pages and single-cache reuse. Frontend fixtures cover presets/custom validation, AND search, immediate stale-request invalidation during debounce, selection/page reset, directory/session reset, background-row preservation and shrinking-total pagination clamping.

The actual integrated App and DiskMap rendered against synthetic native commands at 1440×1400 and 800×1600. Category/size filtering retained the unfiltered map, the caption remained inside its map panel, invalid custom input retained the applied image results, and the narrow page had no document overflow. The final browser console had zero errors/warnings. Temporary preview files/native alias/config, CLI outputs, browser and development server were cleaned up. Main inspected the final desktop screenshot. Independent read-only review approved the frozen candidate after two functional correction rounds (live-refresh row preservation and shrinking-total pagination); no source findings remain. No actual user scan, archive write, permission probe or app restart occurred during these checks.

Final `pnpm desktop:build` passed for the frozen filter candidate. The helper verified the Apple Development-signed host `.app` and stable designated requirement. The bundle was not launched over the user's existing app session; in-memory results and macOS privacy grants were preserved. Public notarization remains deferred. Selectable sorting and Quick Look remain separate follow-ups.


## Persistent list sorting (11 October 2026)

Added sizeDescending/sizeAscending/nameAscending/nameDescending to folder browsing, expanded branches, normal global search and filtered results. Folders remain first in every list regardless of direction. Size ordering uses recorded allocated values; name keys use Unicode lowercase and natural ASCII numeric runs without integer parsing, with fewer leading zeros preferred in ascending order. Raw native path bytes and IDs break remaining ties deterministically. Public list commands accept an optional validated sort argument defaulting to largest first. Membership, metrics, coverage and page caps are retained; map ordering/cache and comparison rankings are independent.

All list sorting runs on blocking workers without snapshot cloning. Completed sessions retain one last-folder ID cache and one global search/filter ID cache keyed by order and metric; pages copy only their bounded ID slice. Running snapshots recompute. Native tests directly compare all four list orders across the three command contexts and verify map JSON remains unchanged. Historical and stale-session action guards remain in force.

A separate bounded private explorer preference record restores the chosen order before the first list request. Atomic temporary-file publication commits at rename; errors before commit retain the previous value, while a post-commit directory-sync failure is reported as a committed save with a durability warning. Native requests reserve monotonic sequence numbers before dispatch, preventing an older queued blocking job from overwriting a newer accepted choice. Preference reads use O_NOFOLLOW|O_NONBLOCK and regular-file validation, so symlink/FIFO/nonregular/corrupt/oversized inputs recover without blocking startup. Missing records default silently. Scan worker settings and saved startup selection are preserved.

The frontend applies order changes immediately, invalidates older responses and row keys, clears pages/selections/branches and preserves filters/search/focus/maps. Only the sort control is disabled while saving. Failures retain the active order and offer retry; successful saves with warnings display those warnings accurately. StrictMode and delayed preference restoration are guarded so stale loads cannot replace a newer choice.

The final single release million-entry fixture permuted filenames to exercise natural sorting rather than a presorted fast path: size sort first query **20.504 ms**, cached page **11 µs**; natural name first query **407.704 ms**, cached page below microsecond timer resolution. Peak process RSS was **259,276,800 bytes** on macOS. Fixture time was 0.58 seconds; compilation was excluded. This synthetic in-memory measurement excludes archive loading, transport and rendering and is not an app-level latency guarantee. Shared engine source remained unchanged after this profile.

Final candidate checks passed: **176 workspace Rust tests** (76 core, 100 native; thirteen opt-in checks ignored), **98 frontend tests** across thirteen files, workspace formatting, changed-frontend Prettier/diff whitespace checks and strict all-target Clippy. New coverage includes four orders, both metrics, groups and ties, Unicode/native invalid UTF-8 names, long numeric runs/leading zeros, capped deterministic pagination, cache replacement/running updates, preference defaults/restoration/malformed/FIFO recovery, failed-write retry, reversed queued jobs, startup list gating, branch/selection/page reset, preserved maps and stale response rejection.

Actual integrated App/DiskMap rendered against synthetic native commands at 1440×1500 and 800×1600. Smallest-first displayed folders first, then 128 MB/256 MB/3 GB files; global image filtering sorted 64 MB/128 MB/256 MB entries, and save failure/retry remained usable. The narrow document matched its 800-pixel viewport; final console had zero errors/warnings. Temporary preview/native alias/config and CLI artifacts were removed, and the owned browser/server were stopped. Main inspected the desktop screenshot. Independent review (configured Sol 6.1 High; runtime identity/effort unexposed) approved after one correction round closing reversed queued-write ordering, committed-warning reporting and FIFO startup recovery. Candidate-matched checks and the unchanged shared-engine benchmark were reused; no reviewer files were edited.

Final `pnpm desktop:build` passed for the frozen sorting candidate. The helper verified the Apple Development-signed host bundle and stable designated requirement. The app was not launched over the user's existing session; in-memory results and privacy grants were preserved. Public notarization remains deferred. Quick Look remains a separate follow-up.


## Native Quick Look (11 October 2026)

Added bundled-main-window-only ID commands and typed monotonic preview status. Only one regular file in a valid completed live session is eligible, including incomplete-coverage results. Unfinished, invalidated and stale sessions remain ineligible. The read-only validator checks recorded file, parent-chain and root-ancestor identities, containment and readability through O_NOFOLLOW|O_NONBLOCK. It does not use Trash’s protected-location blacklist. Filesystem validation runs on blocking workers and refreshes before every presentation attempt after deliberate waits; main-thread URL access checks request/session authority. Scan replacement, saved-result opening, cleanup and restart await native dismissal; window destruction revokes pending authority.

The macOS-only objc2 controller uses Apple’s QLPreviewPanel responder callbacks, preserving Tauri delegates and a weak responder anchor. Retained controller/item/URL objects remain on the main thread. Teardown disables URLs immediately, retains closing-only control during fullscreen transitions, waits for actual disappearance, then restores the responder chain and releases ownership. Native closure advances revisions atomically; stale callbacks cannot close or reopen newer requests. Failure to dismiss returns an error and blocks file recycling.

Quick Look and Space support a single eligible regular file. Repeat Space on the same visible item closes it without reopening. Multiple selections retain normal Space selection behavior; modifiers and interactive/editable/dialog targets are excluded. Selection/navigation/query/filter/sort/session/dialog changes dismiss without auto-follow. Opening state and recoverable errors preserve the displayed results; historical mode explains that a rescan is required.

Quick Look reads current contents by URL after validation. Files can change during the main-thread queue handoff or afterward, so this is not an immutable snapshot of scanned contents. No subprocess preview launcher, copied-file cache, archive change, permission probe or privacy-grant change was introduced.

Candidate checks passed: **187 workspace Rust tests** (76 core, 111 native; thirteen opt-in checks ignored), **115 frontend tests** across fourteen files, TypeScript/Vite production build, workspace formatting, changed-frontend Prettier/diff whitespace checks and strict all-target Clippy. New fixtures cover completed incomplete-coverage eligibility, stale/unfinished/kind rejection, missing/unreadable/replaced files and ancestors, symlink/FIFO substitution, native names, request invalidation, monotonic closure and bounded revisions. Frontend checks cover button/Space eligibility, repeated Space, multiple selections, interactive/modifier exclusions, context dismissal, native closure, stale statuses and failed-close/open errors.

The disposable native smoke exercised real text, image and PDF QLPreviewPanel previews, toggle, capture release, concurrent awaited close, stale requests, responder restoration, delayed presentation with a replaced target, and settled-fullscreen host destruction. The latter removed the Tauri host handle before awaiting dismissal and verified actual hidden state before a simulated cleanup marker. A temporary uniquely identified fixture app also received physical fullscreen/Escape input: first Escape exited fullscreen and second Escape closed. It was removed after exit; the user's app and files were not used. Native NSURL byte round trips, including invalid UTF-8, were checked in memory against Foundation.

Known teardown edge: forced host destruction during fullscreen entry can time out after five seconds. That returns an error and blocks cleanup rather than claiming dismissal; helper process exit removes the panel. Settled-fullscreen host destruction and ordinary native closure passed.

The actual integrated App/DiskMap rendered using synthetic commands at desktop and narrow sizes. Quick Look eligibility, opening/closed/error presentation was checked, with no document overflow and zero console errors/warnings. Temporary preview resources, browser and development server were removed.

`pnpm desktop:build` passed for this candidate. The helper verified the Apple Development-signed host bundle and stable designated requirement. The bundle was not launched over the user's existing app; running results and privacy grants were preserved. Public notarization remains deferred.

Independent reviewer_high (configured Sol 6.1 Max; runtime identity/effort/tier unexposed) approved correction round one with R1–R4 closed: conditional close revision advancement, repeat-Space toggle, hostless dismissal gating and off-thread filesystem refresh after every deliberate retry wait. The reviewer accepted the documented forced-fullscreen-entry timeout as fail-closed and found no remaining blockers. No further source changes followed approval.

## Performance harness and path-capacity candidate (12 October 2026)

The scanner now reserves the final native child-path capacity before appending the name. It keeps the existing node representation, native paths, IDs, descriptor/identity checks, allocation/hard-link accounting, cancellation and archive format. An additive untimed storage report estimates requested snapshot buffer capacities; it explicitly excludes allocator overhead, fragmentation, scan scratch and caller-owned query indexes.

The original source and release executables were frozen before scanner edits. Three candidates were investigated: retained path shrinking, bounded node-vector growth, and exact-capacity path construction. Bounded vector growth was rejected after negligible RSS improvement; exact path construction remains the measured candidate. No compact-tree rewrite or scheduling-policy change was introduced.

Independent reviewer_high source review closed R1–R8 after two correction rounds: private WebKit storage, production cache work in full runs, separate-process persisted preparation, actual final-chart drawing, finite/mode-aware acceptance, complete coverage exports, malformed child handling, and fixture substitution protection. Approval concerns implementation source; it does not establish performance acceptance.

Candidate source checks passed: 193 workspace Rust tests with the benchmark feature (77 core, 116 native; thirteen opt-in checks ignored), 121 frontend tests, 56 Python harness tests, production TypeScript/Vite build, Rust formatting, strict all-target feature Clippy, changed-frontend formatting and diff whitespace checks. The actual isolated Hahakigami UI completed/drew a 100,000-entry fixture and exited cleanly; this smoke is not an acceptance measurement. Both corrected upstream development-signed UI builds passed final-Canvas smokes, and OpenDisk passed cache publication and separate-process persisted preparation on disposable fixtures. The final Hahakigami UI preparation draws a real first scan before same-process/changed measurement; new tests cover delayed event subscription, stale channel/paint replies, original target retention and one-shot rescans. Additional independent review approved CPU drawing completion, two-pass coordination, candidate executable storage checks and coverage diagnostics with total/retained issues. Final preflight verified correct candidate path capacities in both native and packaged variants and matching fixture coverage.

The first cohort is retained with an incident ledger: candidate builds had reused stale scanner artifacts, rescan state ownership rejected preparation, and one occluded packaged completion required intervention. Those candidate results do not establish an optimization effect. Forced clean scanner rebuilds, fresh binary freezes and executable path-capacity contracts precede the corrected cohort. Formal fixture/Home/startup and rescan results, candidate hashes and limitations are recorded with the completed performance report. The user's normal running application, in-memory results, saved library, preferences and macOS privacy grants are preserved; no protected-folder/TCC status probe or grant change was added.

### Completed corrected cohort

The checked-in portable summary (`benchmark-results-memory-summary.json`, removed) contains aggregate medians, coverage counts and binary hashes. Full raw reports and owned build/cache artifacts remain local under ignored `benchmark-results/performance/`; native paths and bounded error samples are not uploaded.

The corrected report (`benchmark-results/performance/suite-corrected-report.json`, local only) and summary (`benchmark-results/performance/suite-corrected-summary.json`, local only) contain **240 successful timed runs, zero failures and zero interventions**, with all **136 source/binary fingerprints unchanged**. Initial runs are separate from five rotating warm rounds. Native and packaged fixture coverage passed as complete, equivalent and stable. Home/startup coverage remains inconclusive: Home has 134 issues per side; startup has 440 issues and eleven excluded branches per side, with unfiltered live path/size/identity differences. No complete-coverage performance acceptance is claimed.

| Full-scan raw memory reduction | Native peak RSS | Packaged sampled owned aggregate RSS |
| --- | ---: | ---: |
| Home | 20.36% | 23.77% |
| Startup `/` | 16.76% | 16.80% |

All full-scan and rescan warm timing medians remain within the 5% regression limit; the largest observed increase is 3.62% for packaged Home same-process rescans. Startup misses the raw 20% memory target. Optimization stops at the three-candidate limit; exact child-path allocation is retained, while path shrinking and bounded vector growth are superseded/rejected. No compact representation or worker-policy experiment was added.

Every observed actual cache mode was full. The clean-folder diagnostic observed publication and journal replay but fell back conservatively on event flags; successful runtime decoding/reuse remains unobserved. Slow packaged same-process rows, including the 87.487-second Home candidate observation, are retained. Scan/aggregation dominates their recorded elapsed time; the cause is unresolved. Native RSS and packaged aggregate RSS are separate measures, and shared/reparented helpers remain unattributed. Detailed medians, coverage mismatches, rejected experiments and external-comparison limitations are in [BENCHMARKS.md](BENCHMARKS.md#corrected-performance-and-memory-results).

The delivered normal bundle at `target/release/bundle/macos/Hahakigami.app` passed `codesign --verify --deep --strict`; its designated requirement exactly matches the frozen baseline normal bundle. The delivery receipt (`benchmark-results/performance/delivery-receipt.json`, local only) records its executable and core-source hashes. The normal bundle was built through `pnpm desktop:build` after a clean scanner rebuild and has not been launched over the user's session. Final documentation/receipt updates do not alter the reviewed candidate source or measured binaries.

## Incremental replay follow-up (13 October 2026)

Same-process and persisted-cache rescans now demonstrate positive metadata reuse
on an existing repository folder, with exact fresh-scan parity. See the
[incremental follow-up](BENCHMARKS.md#incremental-replay-follow-up-13-october-2026)
for observed counts, reproduction commands, event-overlap limits and the distinction
from the earlier all-full benchmark cohort. This is source/native-pipeline verification;
the previously delivered app and its recorded performance results are unchanged.

Validation passed: workspace Rust tests; twelve focused incremental core tests;
existing-folder same-process/persisted reuse; disposable mutation parity; native
external-alias replacement; formatting and diff checks; strict all-target Clippy
with the performance-benchmark feature. Independent reviewer source review found
no blocking defects. General acceleration and cross-version FSEvents behavior
remain unverified; current-host inode preservation was checked explicitly.

### Rebuilt app and larger-folder timings

The normal app was rebuilt with `pnpm desktop:build` and passed strict signature
verification; its designated requirement is unchanged. Its executable remained
unchanged during the separate benchmark. The benchmark-only reuse-counter report
and Python ingestion regression passed eleven focused harness tests, formatting
and an independent protocol review. Benchmark preparation/full/measurement clocks
were checked against both native and frontend completion paths.

The [100,000-entry comparison](BENCHMARKS.md#signed-app-and-100000-entry-rescan-comparison)
and portable summary (`benchmark-results-incremental-summary.json`, removed) record 36
successful timing runs, unchanged source/binary fingerprints, actual reuse in all
incremental samples and repeated exact coverage parity in both layers. End-to-end
rescans are slower on this fixture despite lower scan/aggregation work. Existing
sessions, preferences and OS privacy grants were preserved.

### Optimized rescan delivery (13 October 2026)

Final source checks passed: 80 core and 133 native workspace tests with the benchmark
feature; focused private codec/index/history compatibility tests; strict all-target
feature Clippy; Rust formatting and diff checks; production Svelte checks/build.
Opt-in native checks demonstrated unchanged archive retention and exact memory/
persisted parity on 100,001 entries, mutation during decoding, loader failure/panic
cleanup, late-write rejection of speculative data, external hard-link/clone changes,
and cancellation keeping visible results untouched.

Independent reviewer and reviewer_high checks approved affected cache publication,
manual decoder bounds/checksum/EOF behavior, event/loader lifetimes, metadata-equality
proof, cursor age, and speculative-candidate rejection. Final report completion is
gated by exact stable coverage and unchanged hashes; approval does not itself imply
performance success.

The [optimized results](BENCHMARKS.md#optimized-incremental-rescans-13-october-2026)
and portable summary (`benchmark-results-incremental-optimized-summary.json`, removed) show
72 successful timed runs, stable candidate-source/helper/binary fingerprints and
repeated exact coverage parity. Both native and app incremental modes now beat their
own optimized full-scan medians on the tested folder. The development-signed normal
bundle has the unchanged designated requirement. Existing sessions,
preferences and OS privacy grants were preserved. Speculative transient memory and
cooperative cancellation limits are documented; broader speed guarantees are unproven.


## Safe hard-link cache reuse and whole-GitHub measurement (2026-10-13)

Known groups now participate in the private cache with raw own sizes and global
allocation-owner recomputation. Cached member/subtree/listing changes invalidate
all aliases. Uncached dirty paths refresh every known group, including transient
aliases. New inode groups retain full retry. Replay matches file IDs without requiring
ItemIsHardlink; ambiguous external history refreshes known groups through held
descriptors and rejects private speculation. Missing explicit hard-link IDs and
unsafe history still fail closed. Partial coverage, identity, age and epoch guards
remain intact. Invalidation uses hashed path/ancestor matching.

Independent Max review found and approved correction of a public path-only known
inode/new-alias stale-size case. Visible and transient alias regressions compare
identities, own/accounted sizes, flags and totals with fresh scans. Core: 86 passed,
1 ignored. Native suite: 135 passed, 16 ignored; unaffected evidence was reused after
the focused core correction. Final native internal/external alias mutation and
persisted reload parity passed. External replacement identity coverage and late
speculative-write rejection passed. Strict workspace feature/all-target Clippy,
formatting and diff checks passed. No privacy grants were changed.

The final normal and isolated apps passed the signed desktop helper. The normal app
retained its designated requirement and the user's session stayed intact. All 36
timed whole-GitHub scans had 992,608 entries and zero errors/exclusions. All 24 rescan
requests were actually incremental with 865,211–868,145 reused entries. Eight exports
produced exact within-layer and cross-layer own-metadata parity, with matching
deduplicated root totals and unchanged source/binary fingerprints. These are complete
unchanged-folder checks; native fixtures provide changed-alias evidence. Same-process
timings showed little benefit and persisted reloads were slower. BENCHMARKS.md and
benchmark-results-github-hardlinks-summary.json record exact results and limits.
The interrupted pre-correction cohort was retained separately and not pooled.


## Single reconstruction for hard-link baselines (2026-10-13)

`prepare_rescan` returns the original cache/token without reopening, cloning or
reconstructing when the baseline contains hard links. Final live replay still
flushes/drains, resolves aliases and checks epochs before the existing authorized
core reconciliation. No prepared tree can bypass alias refresh. Other baselines
retain speculation with existing late-write rejection. Independent focused review
approved this source change without findings.

Native suite: 136 passed, 16 ignored. The new regression writes through an external
alias after loading, verifies the original visible baseline remains intact, then
requires exact identities, raw/accounted sizes and hard-link flags against a fresh
scan, plus clean-branch reuse. Native internal/external alias and persisted parity
passed with assertions that no speculative span was recorded. The non-hardlink
native late-write rejection test passed. Unchanged core evidence (86 passed,
1 ignored) was reused. Strict workspace feature/all-target Clippy, formatting and
diff checks passed.

The signed desktop helper rebuilt normal and isolated apps, verified their signatures
and preserved the normal designated requirement. The user's session and privacy
grants were preserved. The final whole-GitHub cohort ran 36 timed observations and
8 separate exports with unchanged source/binary fingerprints. All 24 rescan requests
were truly incremental with positive reuse, no fallback and no speculative phase.
All scans had 992,883 entries and zero errors/exclusions, exact within/cross-layer
own-metadata parity and matching deduplicated root totals. App warm medians were
5.522 s full, 3.611 s same-process and 3.988 s persisted: 34.6% and 27.8% reductions
against this candidate's full scans. BENCHMARKS.md and
benchmark-results-github-alias-deferral-summary.json record method and limits.


## Dense seed preparation and namespace validation (2026-10-13)

The incremental constructor now uses dense validated parent IDs for subtree/listing
invalidation and staging cached children. It retains conservative unknown-path
refresh, all-alias invalidation, final path/identity matching and every held-descriptor
open. No archive/public API changes or worker traversal shortcuts were introduced.
An independent old-rule oracle covers mixed subtree/listing/alias cases and child
order. Max review's duplicate-path collision finding was corrected by validating
unique paths before construction. Duplicate directory records with disjoint children
and duplicate files now trigger full fallback with exact fresh-scan parity.

Final checks: 88 core tests passed, 1 ignored; 136 native tests passed, 16 ignored.
Native internal/external alias mutation and persisted reload parity passed, as did
non-hardlink late speculative-write rejection. Strict workspace all-target feature
Clippy, formatting and diff checks passed. Independent Max review approved R2 closure
and the final candidate. The normal/isolated signed builds passed verification and
the normal designated requirement remained unchanged. User sessions/privacy grants
were preserved.

The whole-GitHub cohort completed 36 timed observations and 8 coverage exports with
unchanged source/binary fingerprints. All 24 rescan requests were incremental with
868,165–868,182 reused entries, no fallback and no speculation. All scans had
996,424 entries, zero errors/exclusions, complete coverage and matching deduplicated allocated totals. Exact within/cross-layer own metadata comparisons
passed. App warm medians: 5.601 s full, 2.949 s same-process, 2.740 s persisted.
See BENCHMARKS.md and benchmark-results-github-reconciliation-summary.json for
measurement scope, diagnostic residual estimates and limits.


## Fresh cached-directory opens with fewer leases (2026-10-13)

Queued names now encode once through a validated private ChildName. Invalid names
remain errors and cannot enter root-only parent borrowing. Cached leaf detection
comes from the same staged entries, not an independent assertion. Taking a listing
before open does not authorize or count reuse; a fresh no-follow read open and
identity check remain mandatory. Only proven cached leaves use a locally owned FD
without a retained-parent lease. Branches and fresh listings keep original leases
and secure pressure fallback. Descriptor peaks continue to measure retained leases.

Final checks: core 90 passed, 1 ignored; native suite 136 passed, 16 ignored.
New tests reject invalid names and preserve raw non-UTF8 bytes; cached leaf inode
replacement and outside symlink cases return incomplete/error results without
cached/outside contents or failed-leaf reuse. Existing held-parent rename, descriptor
pressure, cancellation and alias tests passed. Native alias mutation/persisted parity
and non-hardlink late-write rejection passed. Strict workspace feature/all-target
Clippy, formatting and diff checks passed. Independent Max review approved the
candidate. No stat-only reuse, directory validation skips or scheduler changes were
introduced.

Normal/isolated signed builds verified and the normal designated requirement stayed
unchanged. The user's session/privacy grants were preserved. Thirty-six timed
whole-GitHub observations and eight exports completed with unchanged source/binary
fingerprints. All 24 rescans were truly incremental with no fallback/speculation.
All scans had 999,110 entries, zero errors/exclusions, complete within/cross-layer
own metadata parity and matching deduplicated totals. Fresh relative/absolute open
attempts matched every non-root processed directory. App medians: 5.586 s full,
3.095 s same-process, 3.301 s persisted. No additional speed gain over the prior
cohort is claimed; see BENCHMARKS.md and the portable summary for method/limits.


## Cached validation worker policy (2026-10-13)

The additive core API selects a larger pool only after seed/topology/root/coverage
validation and eligible cached-listing staging. Its callback reports the pool before
starting workers; invalid seeds and newly discovered hard-link retries report/reset
the original full-scan count. Existing explicit-worker APIs retain their behavior.
The app captures public scheduling conditions once and stores a bounded healthy
folder cap. Actual counts are reflected in summaries and saved/cache metadata.
Fresh opens, fstat identity checks, alias invalidation and descriptor budgets remain
unchanged. No worker preference restoration or archive-format change was introduced.

Checks: 92 core tests passed, 1 ignored; 138 native tests passed, 17 ignored.
Large cached-directory regression verifies boosted count, open parity, fresh coverage,
invalid/full-dirty fallback and hard-link retry reset. Policy bounds, retired preference
handling and summary/archive worker-row consistency tests passed. Native alias mutation/
persisted parity and late speculative-write rejection passed. Strict workspace
all-target performance-feature Clippy, formatting and diff checks passed. Independent
Max review approved the stable candidate without blocking findings.

Normal and isolated signed builds verified, with unchanged normal designated
requirement. Whole-GitHub benchmark: 36 timed observations, all 24 rescans genuinely
incremental; eight exact coverage exports matched within/across executables. All
scans had 1,000,670 entries, no errors/exclusions and matching root totals. Full pools
reported three workers and cached pools four; every worker-row count and fresh
non-root open attempt matched. Source/binary fingerprints remained unchanged.
App warm medians: 5.773 s full, 2.568 s memory cache, 2.868 s persisted cache.
See BENCHMARKS.md and benchmark-results-github-concurrency-summary.json for scope,
worker probe, reuse variation and measurement limitations.


## Compact cache writer and publication (2026-10-13)

Private compact saves use lossless Zstd level -1 and a bounded 128 KiB encoder-input
buffer. Cache binary records and decoder acceptance are unchanged;
all checksums, topology/accounting validation, limits, synchronous frame completion,
file/directory sync and archive/row/cursor atomic publication remain in place.

Focused library checks: 18 passed, 3 ignored. Candidate native suite: 141 passed,
18 ignored. Tests cover byte-identical buffered records, short-write failure at
chunk/final-tail boundaries, final-tail decoded byte limits and temp cleanup,
corrupt/truncated/trailing frames and preservation of existing selection bytes.
The full suite found three stale private-selection assumptions; R3 was fixed by
checking permissions on actual cache files, explicitly preserving legacy selection,
and injecting a genuine compressed-budget publication failure while retaining the
previous seed. Independent
Max review approved the corrected candidate. Native alias mutation/persisted parity
and late speculative-write rejection passed. Strict workspace all-target feature
Clippy, formatting and diff checks passed. Unchanged core evidence: 92 passed,
1 ignored; no directory traversal or concurrency changes in this candidate.

Both signed builds verified with unchanged normal designated requirement, preserving
user sessions/privacy grants. Final whole-GitHub cohort has 36 timed observations,
all 24 rescans incremental, and eight exact coverage exports matching within/across
executables. Source/binary fingerprints stayed unchanged. Every scan had 1,003,125
entries, zero errors/exclusions and matching deduplicated allocated totals.
Fresh non-root opens and actual worker reporting were verified. App medians:
5.709 s full, 3.120 s memory cache, 3.217 s persisted; rescan cache publication:
324.710/330.069 ms. The controlled encoding probe improves 204.461 to 179.653 ms
with larger cache archives; whole-app gains versus the prior cohort are not claimed.
See BENCHMARKS.md and benchmark-results-github-publication-summary.json for limits.


## Incremental dirty-work scheduling (2026-10-13)

Validated seed dirty-listing costs and ancestor costs now select ready incremental
work through a max heap; neutral tasks keep LIFO. Sparse immutable hints require
exact path+identity matches and affect ordering only. They neither authorize reuse
nor change fresh enumeration/open/fstat rules. Ordinary scans/full hard-link retries
use the original LIFO queue. Outstanding-work accounting, Condvar wakeups, descriptor
budgets and cancellation remain unchanged; both ready queues drop held references
on cancellation. No additional tasks or skip-validation path was introduced.

Core: 96 passed, 1 ignored. New tests check original full ordering, expensive dirty
branches ahead of already-ready cached tasks, neutral identity mismatch, and ancestor
cost propagation without prioritizing clean siblings. Existing large cached pool,
fresh-open/exact coverage, hard-link retry reset, cancellation, namespace swap and
FD-pressure cases passed. Native: 141 passed, 18 ignored. Opt-in internal/external
alias mutation and persisted parity passed, as did late speculative-write rejection.
Strict workspace all-target performance-feature Clippy, formatting and diff checks
passed. Independent Max review approved the stable candidate without blockers;
previous R1/R2/R3 findings remain closed. Both signed builds verified with unchanged
normal designated requirement; user sessions and privacy grants were preserved.

The paired previous/candidate app comparison completed 16 observations with stable
fingerprints and matching entries/totals, fresh opens and actual worker counts.
Three-warm medians improved memory rescans 2.709→2.384 seconds and persisted rescans
3.178→2.509 seconds; summed worker queue waits fell to 6–7 ms. Final candidate
cohort: 36 timed observations, all 24 rescans incremental, eight exact coverage
exports matching within/across executables. All scans had 1,004,020 entries, no
errors/exclusions and matching deduplicated allocated totals. Every non-root
directory retained its fresh open and actual counts matched worker rows. Source/
binary fingerprints remained unchanged. App medians: 5.591 s full, 2.376 s memory
cache and 2.506 s persisted cache. BENCHMARKS.md and the portable tail summary
record sampling interpretation, paired evidence and host-variance limits.


## Seed uniqueness and dirty-event lookup (2026-10-13)

The preparation candidate modifies incremental.rs only. Raw sibling-name uniqueness
uses validated parent IDs while retaining full child-parent path equality, dense IDs,
kinds and connected topology. Parent uniqueness proves full-path uniqueness by
induction; missing filenames fail closed and normalized duplicate parents/files
still reject. One dirty mark map retains original normalized Path equality and
ancestor processing, with requested/matched markers preserving unknown/transient
alias conservatism. Fresh opens, identity checks, scheduler, descriptor budgets,
cache formats and durable publication are unchanged.

Core: 97 passed, 2 ignored. Full-path validation and listing-eligibility differential
oracles cover normalized separators/dots, raw bytes, duplicate directories/files,
unknown parent/transient paths and existing root/subtree/direct-listing/alias cases.
APFS rejects invalid UTF-8 names, so raw-byte coverage mutates valid in-memory leaf
paths rather than requiring unsupported on-disk fixture names. Native: 141 passed,
18 ignored. Opt-in native alias mutation/persisted parity and late speculative-write
rejection passed. Strict workspace all-target performance-feature Clippy, formatting
and diff checks passed. Independent Max review approved the candidate with previous
R1/R2/R3 findings closed. Normal and isolated signed builds verified with unchanged
normal designated requirement; user sessions/privacy grants were preserved.

The paired previous/current app comparison completed 24 observations, initial plus
five warm per version/mode, with stable fingerprints and equal entries/totals.
Warm medians: 2.368→2.126 s memory and 2.530→2.386 s persisted. Diagnostic serial
residual fell approximately 606–613→384–389 ms; this includes setup/joins and is
not an independently timed preparation phase. Validation-only synthetic release
probe: 103.300→56.036 ms warm median.

Final cohort: 36 timed observations, all 24 rescans incremental; eight exact exports
matching within/across native and signed app. Every scan had 1,005,627 entries,
zero errors/exclusions and matching deduplicated allocated totals. All fresh
non-root open attempts and actual pool reporting were verified. Source/binary hashes
stayed unchanged. App medians: 5.615 s full, 2.226 s memory cache, 2.309 s persisted.
BENCHMARKS.md and benchmark-results-github-seed-summary.json record probe scope,
paired evidence, residual interpretation and timing limitations.


## Alias-resolution set deduplication (2026-10-13)

Only events.rs resolution and a benchmark-only lib.rs timing span changed. The
resolver uses standard Path Hash/Eq with borrowed keys and keep flags, retaining
first native-byte representatives after dropping keys before vector compaction.
Output order changes but no consumer requires sorted resolved vectors: speculative
selection checks emptiness and core invalidation combines commutative flags. Paths
and direct listings stay separate; root-device, shared-inode and uncertain-alias
predicates are unchanged. Native replay sort/history/epoch/bounds/cancel behavior,
fresh directory opens, identity checks, scheduling and durable publication remain.

An independent previous stable-sort resolver oracle covers normalization, raw bytes,
duplicate inputs/aliases, device boundaries, rootless snapshots, mixed hard-link
flags and category separation. Focused event tests: 14 passed, 7 ignored. Main's
performance-feature native suite: 142 passed, 19 ignored. Native alias mutation/
persisted parity, external alias replacement and late speculative-write rejection
passed. Strict workspace all-target performance-feature Clippy, formatting and diff
checks passed. Independent Max review approved the stable candidate; R1/R2/R3 remain
closed. Core source is unchanged; prior core evidence (97 passed, 2 ignored) reused.
Both signed builds verified with unchanged normal designated requirement, preserving
user sessions and privacy grants.

The controlled resolver experiment matched exact normalized sets and representative
bytes for every pair, reducing warm medians 183.093→29.947 and 262.298→31.444 ms.
Paired old/new app: 24 observations, stable fingerprints and matching entries/totals;
five-warm medians improved 2.111→1.963 s memory and 2.345→2.183 s persisted.
Final cohort: 36 observations, all 24 rescans incremental; eight exact exports
matching within/across executables. All scans had 1,006,388 entries, no errors/
exclusions and matching root totals. Fresh non-root open counts and actual worker
reporting were verified. Source/binary fingerprints stayed unchanged. App medians:
5.572 s full, 1.984 s memory cache, 2.139 s persisted cache. See BENCHMARKS.md and
benchmark-results-github-alias-set-summary.json for probe scope and timing limits.


## Private cache validation/encoding overlap (2026-10-13)

Only library.rs changed. A compact-only large-cache gate (nodes≥4,096, captured
workers>1) permits one scoped validator on the immutable borrowed Snapshot while
the main thread encodes with unchanged checksums, limits, frame completion, flush
and file sync. Both outcomes must succeed before atomic rename; row/cursor order
and directory durability remain. The scope joins even on encoding failure. Validator
errors/panics reject publication; spawn failure uses sequential fallback. Small
and single-worker saves stay sequential. No scan fresh-open/identity checks,
traversal, scheduler, cache format or public save API were changed.

Native performance-feature suite: 147 passed, 20 ignored. Tests cover late validator
failure after encoding success/no archive visibility, encoding failure/join/error
precedence, child panic rejection, actual large invalid topology and byte-limit temp
cleanup, roundtrip, gate cases. Sequential fallback
ordering is tested directly; actual OS thread resource exhaustion was not induced.
Strict workspace all-target performance-feature Clippy, formatting and diff checks
passed. Native alias mutation/persisted parity and late speculative-write rejection
passed. Independent Max concurrency/persistence review approved, with R1/R2/R3 closed.
Core source unchanged; previous 97-passed/2-ignored evidence reused. Both signed
builds verified, normal designated requirement unchanged; user sessions/privacy grants
preserved. All validation and durability remain inside cache completion and timing.

Real-cache durable-save probe: 12 byte-identical saved files, all loaded/validated,
290.327→224.096 ms warm median. Paired apps: 24 observations, publication about
329–331→249–253 ms; total memory 2.074→1.974 s, persisted 2.161→2.142 s. Initial
old-app warm-up allocated total differed by 2,506,752 bytes, retained and recorded;
all warm pairs and final observations/exports matched. Final cohort: 36 observations,
all 24 rescans incremental; eight exact exports matching within/across executables.
Every scan had 1,008,628 entries, no errors/exclusions and matching final root totals.
Fresh non-root open attempts and actual pool reporting verified. Source/binary
fingerprints stable. App medians: 5.563 s full, 1.945 s memory, 2.133 s persisted.
See BENCHMARKS.md and the overlap summary for probe scope and timing limitations.

## Layout and inspector polish (2026-10-16)

Current-source inspection and isolated screenshots confirmed that collapsed scan
issues, resizable map/list panels and selection metrics were already implemented. This follow-up keeps those patterns, reserves
a little more width for browsing, keeps coverage Details beside its summary, and
puts the live cleanup queue action before compact Finder/Quick Look controls.
Changing the inspected selection, focused item or scan resets detail scrolling;
metric changes, refreshed metadata and asynchronous totals preserve reading position.

All 266 frontend tests passed, including five new inspector-scroll regression
cases. The production frontend build passed with zero Svelte errors or warnings;
changed frontend formatting and diff whitespace checks passed. The isolated
production-browser smoke passed twelve live cases: light and dark at
1280×820, 900×600 and 1536×1024, with no console/runtime errors. Geometry checks
cover visible identity/path/sizes, action order, retained panel minimums, compact
coverage controls and scroll reset when selecting another item. Existing live
eligibility and cleanup handlers are unchanged. Evidence is in `benchmark-results/2026-10-16-layout-inspector/`.
These checks use synthetic native commands in Chrome, not a live WKWebView session.

`pnpm desktop:build` produced the macOS bundle with the existing Apple Development
identity and passed the helper's deep/strict signature checks. The designated
requirement exactly matches the pre-build requirement. The running user app was
not relaunched; no native scan or privacy-setting changes were performed.

## Parent context and linked hover (2026-10-16)

Existing treemap parent header bands, the focused-branch legend, shared colors and
search-result parent paths were retained. Map captions now identify ancestor names
and expose the full display path. Transient amber hover links a nested map tile to
its closest visible list ancestor, and a list row to its exact map tile or closest
rendered ancestor. An ancestor fallback explicitly says which folder the map shows.
Matching uses scan node identities, not display names or paths; aggregates keep
local feedback without inventing an individual row. Hover does not select, open,
scroll, update the inspector or request another layout. Selection and queue marks
remain distinct. Pending map layouts reject stale hits.

All 274 frontend tests passed, with zero Svelte errors/warnings and clean changed
frontend formatting/whitespace checks. Six final synthetic production-browser hover
cases passed (light/dark at 1280×820, 900×600 and 1536×1024), covering reciprocal
highlighting in both map modes, parent headers, same-named filtered files, unchanged
selection/inspector/scroll/layout requests, and stationary-pointer search resets for
both real nodes and grouped tiles. The six existing live interaction cases also
passed; their unaffected queue/dialog/resize evidence predates the final aggregate
reset guard and was reused. Screenshots/reports are in
`benchmark-results/2026-10-16-map-list-hover/`.

Independent review approved after one correction round: an explicit context reset
epoch clears aggregate hover even when shared hover was already null. These are
isolated Chrome checks using synthetic native commands, not a live WKWebView check.

The signed `pnpm desktop:build` completed and passed deep/strict verification with
the existing Apple Development identity. Its designated requirement is unchanged
from before this build. The running app was not restarted or rescanned, and no
privacy-setting changes were performed.

## Search scope and return to results (2026-10-16)

Source inspection confirmed scan-wide name search and filters, visible active
chips, parent paths and unfiltered maps already existed. Result headings now show
the entire-scan scope and root location; the map caption identifies its unfiltered
focused folder. Opening a result folder retains one return point through deeper
browsing. Back to results restores query, filters, metric, sort, pagination,
selection and scroll, then refreshes through the existing guarded result loader.
Pages/scroll clamp when result membership shrinks. New queries/filters and scan
replacement invalidate the return point; failed archive opening preserves it.

All 285 frontend tests passed, including eight integrated return-state cases and
three rendered-row scroll cases. Production build and Svelte checking passed with
zero errors/warnings; changed frontend formatting and diff checks passed. Six final
result-return browser cases and six hover regression cases passed in light/dark at
1280×820, 900×600 and 1536×1024. The return fixture verifies page two, scroll 1200,
two folder levels, restored query/filter/metric/sort/selection, explicit new-search
invalidation and global scope beside a folder-focused map. Captures wait for both
restored rows and the corresponding map layout. Independent review approved with
no correction rounds. Evidence: `benchmark-results/2026-10-16-search-wayfinding/`.
These browser checks use synthetic native commands, not a live WKWebView session.

Before UI work, nineteen unused benchmark app bundles were moved to macOS Trash
after checking bundle identity and running executable paths. Installed and current
build bundles, benchmark reports and screenshots were retained. Recovery paths are
recorded in `benchmark-results/2026-10-16-old-app-cleanup.json`.

The six existing live UI cases also passed, covering selection summaries, panel
resizing, queue actions, filters and dialog focus. All eighteen browser cases had
zero console/runtime errors; live evidence is retained beside the result/hover
reports.

`pnpm desktop:build` updated the existing current macOS bundle using the existing
Apple Development identity. Deep/strict verification passed and the designated
requirement exactly matches the pre-build value. No extra benchmark app bundle was
created; the installed app was not replaced or relaunched.

## First-use scan and folder choice (2026-10-16)

Source inspection confirmed the header Scan action, sidebar native folder picker,
target registration already existed. The empty
welcome screen now shows the selected location with an explicit Scan action and
Choose folder beside it. The welcome and sidebar picker share one controller;
choosing a folder selects its registered target without automatically scanning.
The redundant header is hidden only on the welcome screen.

The picker waits for target discovery and serializes concurrent actions. Pending
requests cannot replace a changed session, navigation, selection or comparison
view, or update an unmounted app. Cancellation preserves the existing selection;
errors use the existing alert. Deferred setup reopens the existing setup dialog
without confirming access or restarting. Browser previews disable native actions.
Native target authority and Full Disk Access acknowledgement behavior are unchanged.

All 329 frontend tests passed, including 24 first-use cases. Production build and
Svelte checking passed with zero errors/warnings; changed-file formatting and diff
checks passed. Six first-use browser cases passed in light/dark at 1280×820,
900×600 and 1536×1024, each exercising ready, picked, cancel, error, long-name,
setup-needed, missing-location and browser states. They verify keyboard order,
visible controls, explicit selected-target scans and no automatic scan on picking.
The six return-to-results regression cases also passed, with no
console/runtime errors. Independent review approved without findings. Evidence:
`benchmark-results/2026-10-16-first-use/`. These checks use isolated Chrome fixtures
with synthetic native commands; no native picker or scan was opened in the user's
running app.

`pnpm desktop:build` updated the current macOS bundle with the existing Apple
Development identity and passed deep/strict signature verification. Its designated
requirement exactly matches the pre-build value. The installed app was not replaced
or restarted, and no extra benchmark app bundle or privacy change was created.

## Scan progress and completion feedback (2026-10-16)

Source inspection confirmed checking/scanning/caching phases, native cancellation,
issue categories and detailed diagnostics already existed. A shared display now
shows phase-specific guidance and an indeterminate activity indicator, then keeps
a compact completion summary of whole-scan entries, both storage totals and scan
time. Opening a folder or changing the size metric does not replace those totals.
Worker/reuse/accounting details remain in Scan information.

Native state remains authoritative: caching is still running even when a completion
timestamp exists. Cancellation is disabled during cache publication. Running sizes
are labeled provisional, entry counts are not called file counts, and no percent,
ETA or independently advancing timer is invented. Reported time may pause during
checking/finalization; final elapsed time includes the full native pipeline. Only
phase/terminal status enters the polite live region. Reduced-motion preferences
stop the activity animation. Cancelled/failed results remain partial, incomplete
coverage stays explicit, expected exclusions are not failures, and cache retention
warnings do not imply loss of the completed results.

All 342 frontend tests passed, including 13 rendered scan-feedback cases. Production
frontend build/Svelte checking passed with zero errors/warnings; changed-file
formatting and diff checks passed. Six progress browser cases passed in light/dark
at 1280×820, 900×600 and 1536×1024, each covering twelve phase/result variants.
They verify cancellation, early completion timestamps, provisional/final totals,
metric/folder navigation, detail access, reduced motion and visible compact values.
Six existing live-interaction cases and six return-to-results
cases also passed. All eighteen cases reported zero console/runtime errors and
their source hashes match the final candidate. Independent review approved without
findings. Evidence: `benchmark-results/2026-10-16-scan-feedback/`. These are isolated
Chrome fixtures with synthetic native commands; native scans and VoiceOver were
not exercised in the user's app session.

`pnpm desktop:build` updated the current macOS bundle with the existing Apple
Development identity, passed deep/strict signature verification and retained the
exact pre-build designated requirement. The installed app was not replaced or
restarted, and no extra benchmark app copy or privacy-setting change was created.


## Cleanup queue and Trash review clarity (2026-10-16)

Source inspection confirmed persistent location groups, multiple-selection totals,
inherited queued-folder markers, readiness/previous-error reasons, native review
collapse and guarded Trash tickets already existed. The queue page now separates
all candidates from ready items and candidates needing a scan or attention. Each
location keeps its exact path, counts and explicit row status. Review explains that
all ready items from the current completed live scan are included; Remove/Clear
change queue metadata only. Same-named removal controls identify the full path
while their native entry ID remains authoritative.

The confirmation count, item kinds, paths and combined sizes come exclusively from
native-reviewed top-level items. A requested selection count is captured before
asynchronous preparation, so later row selection cannot alter its overlap note.
Folder rows explicitly include all contents, including unscanned contents; partial
sizes and symbolic-link-only moves remain distinct. Sizes describe scan accounting,
not reclaimed space. Long lists wrap full paths, scroll independently and support
keyboard focus/End scrolling, while Cancel remains the initial focus and both
buttons remain visible. A scoped initial-focus option retains the shared dialog
helper's existing defaults; focus trapping includes the review list. Native paths,
tickets, identity/revision checks, commit handlers and original-target rescanning
are unchanged. Failed moves and persistence warnings keep their existing feedback.

All 360 frontend tests passed, including 15 confirmation cases, 20 integrated queue
cases and the delayed-review/changed-selection regression. Production build and
Svelte checking passed with zero errors/warnings; changed-file formatting and diff
checks passed. Six cleanup browser cases passed in light/dark at 1280×820, 900×600
and 1536×1024, each covering mixed statuses/locations, exact-ID removal, ready-only
review, parent-child overlap, a 63-item long-path review, empty and blocked queues.
They exercise safe initial focus, keyboard list/trap/End navigation, cancellation,
restored focus and no native commit. Six existing live explorer interaction cases
also passed; all twelve final cases had zero console/runtime errors and matched
final source hashes. Independent review approved after one correction round for
the count snapshot and keyboard list access. Evidence:
`benchmark-results/2026-10-16-cleanup-review/`. These are isolated Chrome fixtures
with synthetic native commands. Actual Trash operations and VoiceOver were not
exercised in the user's running app.

`pnpm desktop:build` updated the current macOS bundle with the existing Apple
Development identity, passed deep/strict signature verification and retained the
exact pre-build designated requirement. The installed app was not replaced or
restarted; no extra benchmark app copy or privacy-setting change was created.

## Cleanup result receipts (2026-10-16)

Source inspection confirmed aggregate moved counts, failure strings, queue pruning,
persistence warnings and original-target rescanning already existed. The native
result now additionally emits one outcome per reviewed ticket item, identified by
its review index. Both preparation paths preserve this order. Existing command
arguments, native paths, validation, tickets, mutation order, pruning and scan
invalidation are unchanged. Injected native tests verify exact partial/all/none
outcomes, serialized indices/statuses, matching review order, retained receipts
after queue-save warnings and zero callbacks on whole-batch rejection.

The controller freezes the reviewed metadata and validates complete unique indices
and matching counts before attaching item results. Legacy responses can retain
confirmed totals without fabricated item identities. A missing/malformed response
or rejected command remains unconfirmed; it never becomes zero successful moves.
Confirmed receipts survive queue refreshes, rescans, failed rescans and browsing.
Per-report IDs and queue epochs reject stale updates and prevent dismissed feedback
from reappearing. A new prepare/cancel does not discard the prior result.

The summary separates moved items, failed items and candidate records across all
queue locations. Exact reviewed paths and failure reasons remain in keyboard
scrollable details. Queue counts require an accepted nonblocked queue view. Failed
refreshes show Unconfirmed and offer a read-only status retry. Persistence warnings
remain separate: already moved items may still be queued if bookkeeping could not
be saved. Remaining counts are never inferred by subtracting moves. Dismissal only
removes feedback, and no automatic Trash retry is introduced.

All 391 frontend tests passed, including 23 controller/result-mapping cases and
eight rendered-summary cases. The Rust workspace suite, focused injected cleanup
tests, strict all-target workspace Clippy and formatting passed. Production build
and Svelte checking passed with zero errors/warnings; changed-file formatting and
diff checks passed. Six result browser cases passed in light/dark at 1280×820,
900×600 and 1536×1024, each covering eleven result scenarios: partial/all/none,
queue-save warning, failed/pending queue refresh, failed rescan, unknown response,
legacy aggregate response, direct selection and a 63-item result list. The six
existing live explorer interaction cases also passed. All twelve final cases had
zero console/runtime errors and matched the final frontend source hashes. Max
independent review approved without findings or correction rounds. Evidence:
`benchmark-results/2026-10-16-cleanup-results/`. Browser checks use synthetic native
commands; native outcome tests inject callbacks on disposable fixtures. Real Trash
operations, VoiceOver and the user's native session were not exercised.

`pnpm desktop:build` updated the current macOS bundle with the existing Apple
Development identity, passed deep/strict signature verification and retained the
exact pre-build designated requirement. The installed app was not replaced or
restarted; no extra benchmark app copy or privacy-setting change was created.


## Explorer workspace redesign (2026-10-16)

The displayed scan's heading and exact source path now remain tied to its root,
captured live target. Choosing another location retains the
current results and selection, names the next scan explicitly, and leaves the
original location available for rescanning. Explicit target selection controls
native target-ID routing; display-path comparisons only control presentation.
Rescans retain their native session authority.

Completed feedback and coverage summaries use compact layouts with details on
demand. Running, stopping, failed, cancelled, incomplete and cache-warning states
retain their guidance. Both recorded sizes, timing, accounting and diagnostics
remain accessible under Scan information.
The inspector clamps identity text, shows a compact parent location and exposes
the complete path in a keyboard-accessible selectable disclosure. Item or scan
changes reset it; metric and metadata refreshes retain reading position.
History defaults to three recent entries per location plus an older active scan,
with independent expansion controls and unchanged timestamp precision/menus.

All 462 frontend tests passed across 42 files. Production Svelte checking passed
with zero errors/warnings; changed-file formatting and whitespace checks passed.
Twenty-four production-browser cases passed in light/dark at 1280×820, 900×600
and 1536×1024 across live and progress fixtures. Two further
long-name/deep-path cases passed at light 1280×820 and dark 900×600. All 26 cases
had zero console/runtime errors and matched current production source hashes.
Checks include splitter keyboard/pointer/ARIA bounds, row
capacity, full-path keyboard disclosure with pinned actions, source identity
after target selection, and active older history after collapse. The initial
splitter/CSS mismatch was corrected using the established 150px/200px bounds;
the established default split preserves map space and useful list capacity.
Independent review approved after one correction round. Evidence is retained in
`benchmark-results/2026-10-16-redesign/`.

Browser checks use synthetic native commands in isolated Chrome. They do not
exercise native WKWebView, VoiceOver, real scans, filesystem actions or Trash.
Native implementation and command contracts are unchanged. The signed macOS
bundle is built through `pnpm desktop:build`; the delivery receipt records its
signature requirement comparison and source/bundle hashes. The installed app
and its in-memory scan session are left intact.

## Reference layout fidelity (2026-10-16)

The workspace now follows the supplied reference composition: location icon and
search/actions above a flat scan metadata row with a coverage chip, followed by
a white summary strip containing both recorded size totals, current matching
volume availability when known, and the metric switch. Available volume space
is labeled separately from recorded metadata. Coverage retains categorized counts,
pagination, copy controls and keyboard/outside-click dismissal in a bounded
details popover. Running, partial, failed and cache-warning guidance remains.

The inspector uses a branch-colored icon, selectable exact path and explicit
Copy path action, stacked secondary metric, valid scan percentage and contents.
Clipboard completion feedback is fenced by selection/session identity. Home is
a navigation shortcut above locations; recent history retains precise timestamps
and older active entries. Folder actions use a full-width Finder control, and
Quick Look stays available for its eligible regular-file state.

Focused root branches receive teal, blue, lavender and amber by recorded on-disk
rank; map shapes, legend, list bars and folder icons share the same palette.
Metric changes retain colors. Aggregates, uncertain ancestry and ambiguous paths
remain neutral. Map labels use a dark color with at least 4.5:1 fill contrast.
Real map areas remain proportional; the reference raster is not a size model.

All 475 frontend tests passed across 43 files. Production Svelte checking has
zero errors/warnings; changed-file formatting and whitespace checks passed.
Twenty-four final browser cases passed with no console/runtime errors and exact
production source hashes: live reference fixtures and all twelve
progress states, each in light/dark at 1280×820,
900×600 and 1536×1024. Reference fixtures use the image's 763.1/812.6/49.7 GiB
values and Library selection to check hierarchy without real-data differences.
Independent review compared rendered 1536×1024 images directly with the supplied
reference and approved the layout and source. Evidence and signed build hashes
are in `benchmark-results/2026-10-16-reference-fidelity/`.

`pnpm desktop:build` produced the Apple Development-signed bundle and passed
deep/strict verification. Installation verified the receipt's executable hash
and an identical designated requirement, retained a rollback copy, and replaced
only `/Applications/Hahakigami.app`. Queue data and privacy
grants were not modified. The restarted native app's final inspection is
recorded in the installation receipt.

## Mockup typography and icon polish (2026-10-16)

Desktop typography now uses a 28px heading, up-to-32px summary totals, 16px file
names in 44px rows and larger navigation text. Search results reserve 60px for
names and parent paths; virtualization uses the same dimensions as painting.
Sidebar/panel spacing and a 300px maximum inspector align with the supplied
reference. Folder icons have branch-colored layers; disks have a chassis,
platter and front plate; brand and utility marks use consistent SVG geometry.

Treemap names and sizes use white text with a restrained dark edge. Top-level
labels retain consistent 16px/18px sizing; descendants use 13px/15px. Roomy nested
folders reserve a 48px two-line band, while narrow folders retain compact bands.
Crowded deep leaves stay unlabelled, with names and sizes still available through
inspection. Label fitting, clipping, parent hit areas and proportional top-level
areas remain intact. The full-path disclosure is beside Copy path and closes
immediately on Escape even before the native toggle binding is delivered.

All 478 frontend tests passed across 43 files; production checking reports zero
errors/warnings, and formatting/whitespace checks passed. Twenty-four visual
cases passed across three window sizes and both themes, with matching production
source hashes and zero console/runtime errors. Paint probes check white labels
and typography, and geometry checks cover row heights, virtualization, disclosure
bounds, history readability and splitter interaction. Keyboard resizing rebases
on rendered geometry when the container changes before ResizeObserver delivery.
Independent review approved the rendered comparison after these two keyboard
regressions were corrected. Evidence and signed delivery hashes are under
`benchmark-results/2026-10-16-mockup-polish/`.

The normal build helper produced and strictly verified the signed macOS app. Its
designated requirement matches the installed app; installation and native-view
verification are recorded in the delivery/installation receipts.

## Final mockup comparison (2026-10-16)

A second comparison aligns the content viewport to the supplied mockup by using
1536×980 without its 44px native title bar. Header and summary positions, the
61/39 default map/list split, 156px relative bars, 32px file icons and the
32/28/24px metric hierarchy now follow that reference. Inspector headings use
30px type and 56px folder icons; desktop actions use 16px type in 50/48/48px
buttons, with the reference footer inset. The picker, Save and queue actions
have corresponding SVG icons. Single-page lists omit disabled pagination while
retaining their result count; multi-page navigation and clamped offsets remain
covered by tests.

Map labels now use clean white text without the previous dark outline. Narrow
folders reserve two lines where they fit, and label measurement still prevents
clipping. Rounded tile paint retains the original proportional geometry and hit
bounds. The legend is available through a labelled toggle and instructions stay
quiet until an item is hovered. Real scan contents and resulting subdivisions
remain data driven.

All 477 tests pass across 43 files, including pagination clamping, nested header
bounds, canvas labels, legend visibility and keyboard resizing. Sixteen final
reference cases pass with matching source hashes and no browser/runtime errors:
live results in both themes at 900×600, 1280×820, 1536×1024 and
1536×980. Twelve supplementary history/progress cases passed before the final
inspector-footer-only CSS adjustment; their unaffected evidence is retained.
Production checking reports zero errors and warnings. Evidence is in
`benchmark-results/2026-10-16-mockup-fidelity/`; signed build and native
installation verification are recorded in its delivery and installation receipts.

The final Apple Development-signed build passed deep/strict verification and
retained the installed designated requirement. The updated installed app was
opened and visually checked with the 5,322,099-entry Macintosh HD archive.
A rollback copy of the previous bundle was retained.

## Map label insets and corners (2026-10-16)

The map now uses 12px root text insets and 10px descendant insets, with root
baselines at 24/47px and descendant baselines at 22/43px. Roomy parent headers
reserve 64px/56px; compact 28px headers retain at least 40px of child space.
Root children have an 8px inset and nested children 4px. Top-level proportions
remain unchanged; gutter hits resolve to the parent and children remain bounded.
Large roots/children use at most 6px/5px corners, capped at 12% of the smaller
painted dimension, keeping tiny cells rectangular. Clipping and measured text
width use the same new insets and preserve queue-badge clearance.

All 478 tests pass across 43 files; static/build and formatting checks pass.
Sixteen light/dark reference cases pass across four viewports, plus a linked
hover case covering treemap and sunburst. The context fixture now explicitly
opens the optional legend. Reference evidence is reused after that harness-only
change; production hashes match every report. Independent focused review
approved the map correction. Signed delivery and native inspection receipts are
in `benchmark-results/2026-10-16-map-padding/`.

## Map colors and gradients (2026-10-16)

The four leading branch hues now follow sampled reference headers: teal
`#3cb8a6`, blue `#4195f3`, purple `#8b7ef3` and amber `#e2b23c`. Root treemap
tiles have a restrained diagonal sheen around their branch color; nested tiles
blend 10–14% white into the original branch hue. Lightening does not accumulate
with depth. Tiny cells use a solid midpoint, avoiding imperceptible gradient
allocations and keeping dense regions crisp. Fine white child edges complete
the surface treatment. Aggregates retain flat neutral fills; sunburst retains
its flat shading. Map, legend, list bars and icons share branch identities.

All 478 tests pass across 43 files, with zero static errors/warnings and a
successful production build. Seventeen source-matched visual cases pass across
the reference themes/viewports and linked hover, including both map modes.
Focused tests cover hue-preserving descendant shading, no depth washout, neutral
aggregates, palette consistency and zero gradient allocations for 5,000 tiny
cells. Independent review approved the rendered result. Evidence and signed
delivery receipts are in `benchmark-results/2026-10-16-map-gradients/`.

Native screenshots embed the display ICC profile. Color comparisons now convert
that profile to sRGB before resizing, sampling or composition. Older composite
comparisons that discarded the display profile appeared more muted; the new
evidence preserves source captures and uses a color-managed comparison.

The signed update retained its designated requirement and passed strict/deep
verification. Native inspection confirmed the shading with the 5,322,099-entry
Macintosh HD archive restored. The sRGB comparison samples nested teal at
(85,192,177), versus (84,191,176) in the reference; parent samples remain close.
Source captures, conversion script, comparison and installation receipt are saved
with the evidence. Display settings and stored scan/queue data were not changed.

## Independent CLI and MCP (2026-10-24)

The existing executable now dispatches command mode before GUI initialization.
CLI and local MCP sessions share the metadata scanner. Cancelled/failed partial
results remain queryable within the MCP session.
Usage and a generic stdio-client configuration are in the README.

- Rust suites: **435 passed** (213 scanner, 215 native shell, 3 CLI processes,
  4 MCP processes); **22 existing opt-in tests ignored**. The existing native
  home-directory fixtures required execution outside the filesystem sandbox.
  After correcting an APFS-invalid test filename, the process suites passed
  separately. Both documentation-test suites completed with zero tests.
- Frontend: **570 tests passed** across 49 files; Svelte checking reported zero
  errors and warnings. The production frontend build passed through the signed
  app build helper.
- Formatting and strict workspace/all-target Clippy passed. Focused Clippy also
  passed after the final process-test additions.
- Real MCP client checks covered discovery, typed inputs, scans, paginated
  queries, errors, cancellation, partial results, and bounded EOF exit.
  CLI checks included invalid arguments, native path encoding, offset overflow,
  and search pages exceeding 500 entries.
- `pnpm desktop:build` produced a sealed Apple Development-signed app and verified
  its stable designated requirement. The bundled executable passed disposable
  CLI/MCP scan/query checks and clean EOF
  exit without stdout diagnostics.
- Independent Standards and Spec reviews approved source candidate `12dd841`
  with no actionable findings. The approved GitHub workflow configuration and
  default triage labels are in place; feature acceptance is recorded in issue #1.

The running desktop session was not restarted. No installed MCP configuration,
PATH, protected folders, TCC records, privacy grants, or keychain trust/ACLs were
changed or probed. Full Disk Access inheritance for command-mode invocation and
individual MCP client UIs remain unverified. Explicit cancellation and disconnect
are covered; protocol cancellation notifications are not part of this release.

## macOS menu bar and shortcuts (2026-10-29)

- The app menu is built in the frontend with `@tauri-apps/api/menu`: File, Edit,
  View, Go, Item and Window menus with Finder-style shortcuts. Every item reads
  the same `app.can` availability flags as its on-screen button and is disabled,
  never hidden, when unavailable.
- Move to Trash (⌘⌫) always opens the Trash confirmation. Go and Item items are
  disabled while a text field has focus, and all app commands are disabled while
  or a dialog or the Filters popover is active.
- Frontend: **609 tests passed** across 51 files; Svelte checking reported zero
  errors and warnings. Independent Standards and Spec reviews ran; their
  findings were fixed before commit.

- `pnpm desktop:build` produced a verified Apple Development-signed app. Manual
  checks passed: with the search field focused, ⌘⌫, ⌘↑/⌘↓ and ⌘[ edit text
  instead of running commands; Find, navigation, Quick Look, Trash
  confirmation (with the menu quiet behind it), View toggles and the system
  Window menu all behave as specified.
