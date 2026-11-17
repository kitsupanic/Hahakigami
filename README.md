# Hahakigami

A local disk-space analyzer for macOS, built with Rust, Tauri 2, Svelte 5, and TypeScript. Explore storage through a size-sorted tree, treemap, or sunburst; inspect files and move a reviewed selection to macOS Trash.

Hahakigami runs on macOS 14 and later. See [benchmark results and methodology](docs/BENCHMARKS.md) for the measured OpenDisk comparison and full-clone accounting cost. Windows and Linux releases are future work.

## Install

Download `Hahakigami-<version>-arm64.zip` from the [latest release](https://github.com/kitsupanic/Hahakigami/releases/latest), unzip it and move `Hahakigami.app` to Applications. The build is for Apple silicon Macs; on Intel, run from source.

The release is ad-hoc signed and not notarized, so macOS blocks the first launch. Open the app once, then go to **System Settings → Privacy & Security** and choose **Open Anyway**. Alternatively, clear the quarantine flag:

```sh
xattr -dr com.apple.quarantine /Applications/Hahakigami.app
```

Each release has a new ad-hoc identity, so Full Disk Access must be granted again after updating.

## Run from source

Requires macOS 14+, Xcode command-line tools, a stable Rust toolchain, Node.js 22.12+ (Node 24 tested), and pnpm 11. Dependencies are pinned in `Cargo.lock` and `pnpm-lock.yaml`.

```sh
# Install Rust if needed using the official installer: https://rustup.rs/
source "$HOME/.cargo/env"
pnpm install --frozen-lockfile
pnpm tauri dev
```

A browser-only preview is available with `pnpm dev`; filesystem operations require the desktop app. No accounts, analytics, remote content, or filesystem payload uploads are part of the app.

Only one desktop app instance may use the same local data directory. Quit the installed app before starting a development copy; a second instance refuses to start to protect the cleanup queue and rescan cache. The lock is released automatically when the app exits.

Build a local application bundle:

```sh
pnpm desktop:build
open target/release/bundle/macos/Hahakigami.app
```

The bundle targets the current Mac architecture. The local build helper uses one existing valid **Apple Development** identity from your keychain and verifies the sealed bundle and stable designated requirement. It does not create/import a certificate, change keychain permissions, grant Full Disk Access, or upload for notarization. If multiple identities exist, select one with `python3 scripts/build_macos.py --identity CERTIFICATE_SHA1` (list identities with `security find-identity -v -p codesigning`).

If no development identity is available, `python3 scripts/build_macos.py --adhoc` explicitly builds ad-hoc; macOS may require permission renewal after every rebuild. Prefer the development-signed `.app` for Full Disk Access testing; the raw `tauri dev` process has a different identity. Public distribution signing, notarization, installers and universal packaging remain deferred.

The near-black Guardian portrait in `src-tauri/icons/source.png` is the app icon's source artwork. To regenerate the rounded macOS icon, sidebar mark, and favicon:

```sh
icon_output=$(mktemp -d)
swift src-tauri/icons/make-icon.swift src-tauri/icons/source.png "$icon_output/app-icon.png"
pnpm tauri icon "$icon_output/app-icon.png" --output "$icon_output/generated"
cp "$icon_output/generated/"{32x32.png,128x128.png,128x128@2x.png,icon.png,icon.icns} src-tauri/icons/
```

## CLI and local MCP

The application executable also runs independent metadata scans without opening the UI. The desktop app can remain open or closed.

Two commands are available: `scan` and `mcp`. `scan` prints the finished result, and nothing is retained after the process exits. To browse a scan's results, use the MCP server.

After building the app, run its bundled executable directly:

```sh
app="$PWD/target/release/bundle/macos/Hahakigami.app/Contents/MacOS/hahakigami"
"$app" --help
"$app" scan /path/to/folder
```

Successful commands write one JSON value to stdout; diagnostics go to stderr. Exit codes are `0` for success, `1` for an operation failure, `2` for invalid command arguments, and `130` after Ctrl-C requests cancellation. Help and version output are plain text. Ctrl-C returns the latest available status; an in-flight filesystem read may still be stopping when the process exits.

Lists use `items`, `total`, `offset`, and `limit`; the default page size is 50 and the maximum is 1,000. Children and search results sort folders first, then largest allocated size. Search matches filenames case-insensitively and accepts 1–1,024 bytes. Node IDs belong to one scan. Paths are display strings; `pathKey` preserves native path bytes through percent encoding. Recorded logical/allocated bytes, partial coverage, accounting warnings, and intentional exclusions remain distinct. Sizes do not predict reclaimable space. Issue-page `total` counts retained details; `coverage` also reports event and stored-detail counts when records were truncated.

For an MCP client that supports local stdio servers, configure the exact executable path and the `mcp` argument. For example, adjust this path to your bundle location:

```json
{
  "mcpServers": {
    "hahakigami": {
      "command": "/Applications/Hahakigami.app/Contents/MacOS/hahakigami",
      "args": ["mcp"]
    }
  }
}
```

The server exposes `scan`, `status`, `cancel`, `summary`, `node`, `children`, `search`, and `issues`. Inputs use `node_id`; responses use camelCase field names. `scan` waits for completion while `status` and `cancel` remain available concurrently. One scan runs per server session. Every query reads that session's current/latest result, including cancelled or failed partial results; results end with the session. Disconnecting or interrupting the server requests cancellation. No protocol-level request-cancellation guarantee is made; use the `cancel` tool.

Command mode uses the same scanner as the app. Scanning reads metadata; it does not preview contents, move files, control the GUI, or host a network endpoint. No client configuration or PATH installation is changed. macOS still controls filesystem access: command mode does not open the desktop Full Disk Access dialog or apply its scan block, and invoking the bundled executable is not proof that it inherits the GUI's Full Disk Access grant. Check coverage and issue results for denied locations. No protected-folder probes or privacy-grant changes are performed.

## Explore storage

1. The sidebar's **Full Disk Access** row shows its status: On, Off or Unknown. If it is Off when you start a scan, a dialog explains that macOS would ask about each protected folder during the scan. **Open System Settings**, add and enable the exact app shown under **Privacy & Security → Full Disk Access**, then return to Hahakigami; if macOS has not applied the change yet, choose **Relaunch**. **Scan anyway** scans without it until the app quits. Unknown never blocks a scan, and nothing else is blocked by the status.
2. The welcome screen shows the selected location and a **Scan** button. The startup disk is selected at launch. Choose an attached volume under Disks in the sidebar, or use **Choose folder** to select another location. Picking a folder updates the selection; press **Scan** when ready. If Full Disk Access is off, **Scan** opens its dialog. **Scan all disks** (on the welcome screen, in the sidebar's Disks header, and in the File menu) starts a fresh scan of every local disk and skips network shares; a disk that is already scanning is cancelled and scanned again. While two or more scans run, the Disks header reads **Scanning · N of M done**.
3. Switch between **On disk** and **Logical**, and between **Treemap** and **Sunburst**. Expand folders in the tree or double-click to focus one; breadcrumbs navigate back.
4. Search names across the scan, select items to inspect, or reveal them in Finder. Cmd-click adds/removes items from the selection.
5. After a scan completes, review the exact paths in the Trash confirmation. Cancelling leaves the files in place. Successful cleanup invalidates the old results and starts a fresh scan of the original location.

The toolbar shows the displayed scan as a folder breadcrumb; the last crumb is the current folder and earlier crumbs open their folders. **Back / Forward** restore visited folders and their search or filter context within that scan; **Enclosing Folder** (⌘↑) opens the parent folder and stops at the scan root. The sidebar buttons hide or show Locations and the explorer pane while retaining their contents. Navigation history resets when changing scans, and sidebar visibility resets when reopening the app. Choosing a disk or folder in Locations (or **Choose a folder**) sets the scan target without changing those results or starting a scan. The toolbar action reads **Rescan** for the displayed source or **Scan** for another target (its tooltip names the location); pressing it starts the scan and, while a scan runs, it becomes **Stop**. A status line under the toolbar shows the scan's size and entry totals, which open the scan information. The inspector keeps long names compact, stacks the recorded sizes, scan percentage and contents, and provides **Copy path**. Expand **Full path** to select the complete path with the keyboard.

If macOS still displays Documents/Desktop/Downloads prompts, quit Hahakigami, remove its old Full Disk Access entry, add the exact `.app` shown in the dialog again, enable it, and reopen. Rebuilt ad-hoc applications can have a different code identity despite the same filename; the local signed build avoids that specific problem. Apple does not provide a public Full Disk Access status API, so the status comes from opening (never reading) a few files only Full Disk Access can open, such as `TCC.db`; protected folders are never probed. See [ADR 0002](docs/adr/0002-detect-full-disk-access-with-silent-probes.md). See [Apple's stable-signing guidance](https://developer.apple.com/forums/thread/663889) and [FDA API limitations](https://developer.apple.com/forums/thread/114452).

Scanning reads filesystem metadata, not file contents. Explicit Quick Look previews read the selected file’s current contents. Cloud placeholders are not opened for download. One scan runs at a time; cancellation preserves a clearly labeled partial result. The interface acknowledges cancellation immediately; an in-flight operating-system filesystem read may delay the final stop.

The scan summary distinguishes checking previous metadata, scanning, stopping and finishing. Running totals are provisional; no percentage or time estimate is shown. The compact metadata row shows the recorded size, entry count, timestamp and read-only status. Expand its size total for both whole-scan size totals and accounting details; **On disk / Logical** above the map chooses the displayed size metric. When volume discovery has current availability for that exact location, it appears separately as **Available on volume**. **Scan information** retains full size details, worker diagnostics, rescan reuse and accounting details. Cancelled/failed scans remain partial, and coverage issues stay beside the summary. The reported scan time can pause during checking or finishing; the final time includes the full scan pipeline.

Scans automatically choose a worker count using scope and public power/thermal conditions. Volume roots use half the active CPU threads, bounded to 4–8. Folders with at least 12 explicitly known active and hardware threads, nominal thermal state and Low Power Mode off use one third rounded up, capped at five. Other folder scans retain one quarter, bounded to 3–5. Counts stay within CPU and scanner limits. On a healthy 14-thread Mac, this selects seven workers for a volume and five for a folder.

Low Power Mode and thermal pressure apply the existing reductions to the conservative scope count; the healthy-folder boost is disabled. Low Power Mode halves that count. A fair thermal state subtracts one; serious or critical temperature halves the remaining count, always keeping at least one worker. Conditions are captured when a scan starts. Healthy folder rescans with at least 4,096 reusable directory listings can use up to four validation workers (or their higher normal count), bounded by available CPUs; full scans and fallback retries use the normal count. Every cached directory still requires a fresh open and identity check. macOS chooses which P/E cores run the work. The progress panel and worker diagnostics show the actual count used.

No app Settings tab, worker selector or calibration is needed. Every new scan uses hardware Auto and default macOS scheduling, ignoring older Manual, QoS and calibration preferences. Older preference records are left in place. Full Disk Access status remains available in the sidebar. The automatic count is a bounded heuristic and does not promise the fastest scan for every disk or workload.

Expand **Worker diagnostics** under **Scan information** to inspect each worker's entries, attempted folders, processing/CPU time, metadata time, lock/queue/progress waits, and directory-open counts. **Copy worker report** copies these numbers as JSON without file paths. CPU time appears when a worker exits; phase wall times overlap CPU time and are not CPU utilization. Worker IDs and reported QoS do not identify the actual P/E core used.

The right pane has **Files & Folders**, **File Types**, and **Details** tabs. Selecting a file or folder in the map updates the shared selection while keeping the active tab. Switching tabs retains the file list’s expanded folders and scroll position. Visualization controls sit directly above the map, separate from the breadcrumbs. **File Types** lists scanned regular files by extension, with size, share and count, scoped to **Entire scan** or **Current folder**. Select a type to highlight its matching file tiles and exact type groups. Folder ancestors keep their normal appearance. **Color → File type** shows major files and groups deeper files by their actual extension within a compact folder hierarchy, so the disk overview retains useful type colors. Group labels include their scanned file count; clicking a group highlights its extension without selecting files for actions, and double-clicking opens its containing folder. Up to twelve extensions per retained folder receive separate groups, with remaining types, folder metadata and other items in a neutral remainder. **Show matching files** opens an exact extension filter within the chosen scope and gives the file list more room. Search names stays inside that scope until the filters are cleared; the search field identifies the folder when scoped. Type totals exclude directory metadata and links, so they may differ from whole-scan sizes; partial coverage stays labeled. Switching tabs preserves the current file selection.

The file list supports arrow keys, Home/End and Page Up/Down across virtualized rows. Shift extends selection; Command/Control plus an arrow moves focus without changing selection, and Command/Control+Space toggles an item. Plain Space retains Quick Look behavior for eligible files.

The File types panel renders only the visible part of large extension lists. Search still covers every type, and arrow keys, Page Up/Down, Home and End navigate the full list. Switching the explorer tabs preserves its search and scroll position. Scan-wide statistics stay cached alongside the most recent folder statistics, avoiding another full metadata aggregation when returning to the scan scope.

## Quick Look previews

Select one regular file from a completed live scan and choose **Quick Look** beside Finder reveal, or press Space on its focused row. Repeating Quick Look for the same visible file closes it. Escape and the native close control also close the panel. With multiple selected entries, Space retains its normal selection behavior.

Selection, navigation, query/filter/sort changes, dialogs and session changes dismiss the preview without automatically following the new selection. Completed scans with coverage issues remain eligible. Unfinished or invalidated scans, directories, symlinks and special entries cannot preview.

Quick Look displays current contents, using the native fallback for unsupported formats. The backend checks recorded file and ancestor identities and readability before presentation. The URL remains path-based: Quick Look may read after validation, so a preview is not an immutable snapshot of the scanned file. Opening errors preserve scan results.

## Accounting and boundaries

- Logical sizes include data and resource forks for regular files on macOS; reported allocated sizes describe storage reported by the filesystem. Hard-linked allocated storage is counted once, with every path retained for browsing.
- Verified full APFS clones share their data-fork allocation once within the scan. Hard links are reconciled first, then shared data is assigned to the smallest native path among distinct cloned files. Each file keeps its resource-fork allocation and raw filesystem-reported allocation. The inspector explains attribution; a zero-attributed clone remains in the tree and search results.
- Clone metadata is queried automatically relative to held directory descriptors without opening contents. Before deductions, files and ancestor identities are checked again from the original held root. Missing, changed or inconsistent evidence keeps raw allocation and displays a separate accounting warning. A clone whose other members are outside the scan keeps its allocation. These observations are not an atomic filesystem snapshot.
- Partial clone sharing, snapshots, purgeable space, filesystem metadata, and unreadable locations are not completely attributed. Displayed sizes do not predict how many bytes Trash will reclaim; moving to Trash does not immediately free those bytes.
- Startup scans include System/Data namespace entries and Data-only subtrees. Verified firmlink mirrors are excluded. Other mounted filesystems are separate targets; system sibling volumes are not merged into the startup result.
- Expected scope/mount exclusions are listed separately from scan issues and do not mark results incomplete. Excluded folders show **Not scanned**, not an empty-folder size. **Show scan details** lists retained paths and reasons in pages of 100. Issue counts are events, not a count of unreadable files; one directory failure may affect an entire branch. Each category retains up to 10,000 details and reports truncation when needed.
- Permission denials in conservatively recognized macOS system locations appear as a neutral **Restricted system locations** notice. Other failures remain prominent. These are presentation categories: scanning coverage, recorded failures and incomplete totals are preserved. Details separate restrictions, other issues and intentional exclusions.
- **Copy errors** beside the scan-details button copies all retained scan issues as formatted JSON, including paths, reasons, native error codes and scan state. The report gives total/copied/omitted counts when details are truncated. Expected exclusions are not included.
- Zero-sized entries remain in the tree. Maps are bounded at 5,000 entries; grouped small items have no file actions. Drill down to inspect their full contents.

## Persistent cleanup queue

Select items from a completed live scan and choose **Add to cleanup queue** to collect candidates while browsing. The **Cleanup queue** sidebar entry opens a review page grouped by original scan location, with separate counts for all candidates, ready items in the current scan, and candidates needing a scan or attention. Each row shows its readiness, exact path, recorded sizes and any previous cleanup error. Returning to the explorer preserves navigation, filters and selection. The existing **Move to Trash** action remains available for immediate reviewed cleanup.

The queue survives scan changes and app restarts. It stores at most 1,000 candidates using lossless native paths; duplicate paths are ignored and queued folders replace covered descendants. Recorded sizes describe allocation at scan time, not reclaimable space. Removing candidates or confirming **Clear queue** changes only the queue, never the files.

**Scan this location** starts a normal scan of a candidate's original location. **Review ready items** reviews candidates matching the current completed live scan's root, item and ancestor identities. Candidates from other locations, missing files or replaced paths cannot be cleaned up from the queue. Remove and deliberately add a replacement if you want to queue it. Only starting a scan depends on Full Disk Access status; browsing, adding, removing and moving candidates to Trash do not.

The final confirmation shows the exact reviewed paths, item kinds and combined scan-recorded sizes. A selected parent folder includes its selected descendants, so the final item count can be smaller than the selection count. Moving a folder includes all its contents, including contents that were not scanned; sizes do not predict space reclaimed. Long review lists support keyboard scrolling, with Cancel initially focused. Queue review includes all ready items from the current scan; other candidates stay queued.

Confirmation uses the same two-minute, one-use review and native Trash safeguards as direct cleanup. Cancelling keeps candidates. Successful moves remove covered candidates and rescan the original active location; failures remain queued. Queue persistence errors are reported separately from successful Trash moves. Unreadable or unsupported queue records require an explicit clear before modifications, rather than being silently overwritten. No permanent-delete fallback or automatic cleanup is provided.

After cleanup, a summary separates items moved to Trash, failed moves and the verified queue count across all locations. Expand **View moved and failed items** for exact reviewed paths and failure reasons. The summary stays available while scan results refresh and when returning to the queue. A queue-save warning can leave moved items listed as candidates; the summary reports the actual queue view rather than subtracting successful moves. Failed queue refreshes show **Unconfirmed** with **Retry queue status**, which never repeats Trash. If the cleanup response itself cannot be confirmed, no moved/failed count is invented. Dismissing the summary changes only the feedback.

## Incremental rescans

Fresh scans automatically try a separate, private rescan cache. Only complete, clean results from local folders with explicit nonshared accounting evidence are eligible: startup-disk scans, unreadable or excluded locations, and scopes containing the optimization cache use full scans. Shared clone baselines, potential partial sharing, and unresolved clone checks require a full scan. Fresh sharing discovered during an incremental attempt discards the private candidate and retries once against the same held root. Older optimization-cache records are rebuilt. Known hard-link groups can reuse cached metadata while retaining every path and counting allocated storage once. Matching root identities, boot/journal UUIDs and a recent original full-scan timestamp are required. A full scan is forced after an hour or whenever event history is unavailable, ambiguous, dropped, oversized, or affected by namespace/shared-allocation changes.

The scanner reuses clean directory listings identified through public macOS FSEvents history. Incremental scheduling brings expensive dirty branches forward using validated workload hints, so fresh enumeration can overlap cached validation; ordinary full scans retain their existing traversal order. Every directory still opens through held-descriptor identity validation. Dirty aliases refresh every cached member of their inode group. Newly discovered hard-link groups trigger a full retry using the same held root. Incremental candidates and retries keep topology private until one final result is published, so browsing and cleanup IDs cannot change during retries. Cancellation publishes a labeled partial result. Progress indicates cache checking, scanning, finalization, reused metadata counts and full-scan reasons. Cancellation is unavailable during the short cache-finalization phase; session changes and cleanup remain blocked until it finishes.

The optimization cache uses checksummed, bounded binary node records with native path bytes and a small metadata header. Private cache writes use lossless fast compression and a bounded input buffer, retaining full validation and durable publication; Older optimization caches are discarded and rebuilt. This cache is automatic and disposable. It keeps the latest cache for at most eight locations, within a 512 MiB compressed-archive budget, and removes incomplete cache-owned files left by interrupted writes. A failed cache update preserves scan results and reports a nonfatal warning. Scans include journal/cache overhead in their native elapsed time; standalone engine benchmarks still measure the core scanner.

History replay watches the selected local filesystem mount. Extended event inode IDs invalidate cached files changed through outside-folder hard-link aliases; hard-link events without a usable identity still require a full scan. File events are matched by inode even without a hard-link flag. Other ambiguous external alias history refreshes every cached hard-link member through held descriptors and discards speculative results. Clone events invalidate their reported paths under the current per-file allocation model. Detailed directory creation/metadata events refresh directory listings without discarding unchanged descendants; renames, removals and ambiguous history remain conservative. Disk-cache metadata is checked before decoding. One historical stream stays alive while decoding, then flushes and drains queued changes. Baselines containing hard links wait for final alias resolution before constructing the tree once. Other scopes may privately reconstruct a candidate during replay. The candidate is used only after final epoch checks and a replay with no relevant changes; otherwise reconciliation runs against the original validated baseline. Unchanged metadata can retain the existing archive and atomically advance its cursor without extending the original full-scan age. This proof requires an exact fresh root listing and validated reuse of every descendant listing. Speculation temporarily retains additional snapshot memory; cancellation cannot interrupt an in-flight decoding/reconstruction join. Reuse is opportunistic and does not guarantee faster end-to-end scans. Unresolved same-volume hard-link events and required historical overlap can force conservative full scans even for a quiet selected folder. Small trees may cost more to check and cache than to enumerate fully. Read [verification notes](docs/VERIFICATION.md) for candidate-matched fixture evidence and performance limits. No new Full Disk Access verification or protected-folder probes are introduced.

## File category and size filters

The explorer's **File category** and **Size range** controls narrow entries across the entire scan and combine with name search. Choose a binary-size preset (100 MB+, 1 GB+, 10 GB+) or a custom minimum/maximum with a shared unit selector, then apply the custom range explicitly. Bounds include the entered endpoints and use the selected allocated or logical metric. Invalid custom ranges keep the previously applied range.

Search and filters show a flat, paginated result list labeled **Entire scan**, with the scan's location beneath its heading. Removable category and size chips keep applied values visible when the filter popover is closed; removing one preserves the other, and clearing filters preserves name search. Size chips name the active metric and use exact binary units or byte boundaries. Global search and filtered results show parent paths beneath names. List bars follow the focused map's branch colors, with neutral bars for results whose branch is unavailable or outside that focus. Maps and scan totals remain unfiltered; their caption identifies the map's focused folder. Category groups use native filename extensions: images, videos, audio, documents, archives, applications and other regular files. Applications include `.app` directories and `.pkg`/`.dmg` regular files. All entries also permits ordinary folders, symlinks and special entries; folder size filtering uses recorded aggregate sizes. The root and excluded placeholders are omitted from active filter results. Category detection reads no file contents and follows no links.

Changing filters or metric resets the page and clears selection. **Clear filters** preserves name search. Opening a directory from results switches to its ordinary tree view and offers **Back to results** above the folder heading. This restores the original search, filters, size metric, sort order, page, selection and scroll position, including after browsing deeper folders. Results are queried again from the same scan; pages and scrolling are clamped if fewer matches remain. A deliberate new query or filter replaces the return point. A new scan clears it; it is not retained across restarts. Filters reset for each new scan and are not persisted. Recorded coverage notices remain in force. Existing allocation and hard-link accounting are unchanged.

## List sorting

Use **Sort order** above the file list to choose largest/smallest size or natural case-insensitive name order (file2 before file10). Folders remain first in every list, including search and filtered results. Size sorting uses the current allocated/logical metric; maps retain their independent ordering.

The chosen order is remembered across navigation and app restarts. Changing it clears selection, expanded branches and pagination while retaining filters/search and focused folder. The order applies immediately while only the sort control waits for the preference write. A failed save leaves the active order usable and offers **Save current order** to retry; missing preferences default to largest first, and unreadable/malformed preferences show a recoverable warning. Preferences are stored separately from scan settings.

## Architecture

- `crates/disk-scan`: Tauri-independent metadata scanner, compact indexed tree, native macOS bulk reader, portable Unix descriptor reader, cancellation, tests, and release benchmark examples.
- `src-tauri`: native target registry, one active scan session, streaming channels, paginated queries, Finder/Trash adapters, and command permissions.
- `src-tauri/src/automation.rs`, `cli.rs`, and `mcp.rs`: independent metadata scans and queries behind command-line and local stdio interfaces.
- `src`: Svelte UI and per-session reactive controllers, virtualized tree rows, navigation and selection, Canvas rendering, and worker-based layouts.

Native paths and filesystem identities remain authoritative in Rust. Action requests carry scan/node IDs, not executable shell strings or frontend paths. Custom command permissions apply only to the bundled main window; no remote origins, shell plugin, or unrestricted filesystem plugin are enabled.

Scan trees use aligned 80-byte records, pooled native filename bytes and pooled child IDs. The root path is stored once; other paths are reconstructed through recorded parents with bounded reusable buffers. Every node still retains its device/inode/type identity and allocated/logical size values. Display names remain separate from native path authority. Records with unusual path spellings or display names use sparse lossless overrides, while the optimization-cache format remains compatible.

Storage diagnostics distinguish retained snapshot buffers from temporary reuse and query estimates. Requested capacities exclude allocator overhead; reuse index estimates also exclude hash-table controls, and the largest materialized listing is not a concurrent memory peak. Process RSS is measured separately. Validation and codec loading use bounded parent-prefix cursors without retaining a second full-path tree.

Cleanup rejects scan/volume/home roots and protected system locations, collapses overlapping selections, and uses a short-lived one-use review ticket. Item and ancestor identities are checked again immediately before native recycling. macOS's path-based Trash API is not atomic with those checks; concurrent filesystem renames can still race. There is no permanent-delete fallback.

## Verify

```sh
cargo test --workspace
pnpm test
pnpm check
pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The native recovery test is deliberately opt-in. It creates a disposable fixture, moves that fixture to Trash, and restores it:

```sh
cargo test -p hahakigami native_trash_and_recovery -- --ignored
```

[Verification notes](docs/VERIFICATION.md) record packaged-app checks and remaining limitations. [Benchmark tooling](docs/BENCHMARKS.md) includes worker sweeps, a million-entry in-memory fixture, and a pinned OpenDisk full-scan engine harness with incremental caching disabled.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report security problems privately as described in [SECURITY.md](SECURITY.md).

## References and license

Inspired by [WinDirStat](https://windirstat.net/), [Radix](https://github.com/colinvkim/Radix), and [OpenDisk](https://github.com/137137137/OpenDisk). The application scanner/UI are implemented independently. The comparison harness compiles pinned upstream OpenDisk sources in an ignored cache; its license is retained in `scripts/OPENDISK-LICENSE.txt`.

Hahakigami is licensed under [MIT](LICENSE).
