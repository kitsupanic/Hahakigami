---
status: accepted
---

# Ship a sandboxed Lite edition from the same source

The Mac App Store requires the App Sandbox, which forbids two things the app relies on: reading a disk the person has not picked, and the `scan`/`mcp` subcommands. Rather than sandbox the only app, or stay off the Store, we ship two **Editions** from one codebase. **Hahakigami** (the Full edition) stays as it is: Developer ID, notarized, downloaded directly (ADR 0006). **Hahakigami Lite** (the Lite edition) is sandboxed and distributed through the Mac App Store. Lite lacks only what the sandbox forbids, is free, and never mentions the Full edition inside the app.

## Decisions

- **Separate apps.** Lite uses bundle ID `app.hahakigami.lite`, can be installed next to the Full edition and shares no saved state with it (the sandbox moves its data into its own container anyway). Both editions ship from the same tag with the same version number.
- **Allowed locations.** In Lite, every Scan target is an Allowed location or lies inside one. Every disk stays in the Disks list; a disk that isn't allowed yet offers "Allow access…", which opens the Open dialog already pointed at that disk. A folder picked through Scan folder becomes an Allowed location. Allowed locations are remembered across launches. When one no longer resolves, it quietly goes back to "Allow access…". "Remove access" forgets the location but keeps any scan already on screen. In Lite, Scan all disks scans only allowed disks and reports how many it skipped. A Cleanup queue entry outside every Allowed location just needs a scan, under the existing Matched scan rules.
- **Full Disk Access works the same in Lite.** The spike showed that Full Disk Access widens what a sandboxed scan reads, so Lite keeps the Full Disk Access status, prompt and settings link. Under the sandbox `$HOME` is the app's container, so probes that name the home folder must use the person's real home. The settings link and Reveal must not shell out (`/usr/bin/open` is blocked); they use `NSWorkspace`.
- **Automation scans are compiled out.** A `lite` Cargo feature removes the `scan` and `mcp` subcommands and the `rmcp` dependency from the Lite binary.
- **One seam.** A single `edition` module answers the questions where the editions differ. The frontend reads those answers through one command. Apart from removing `cli`/`mcp` and the code that saves and restores Allowed locations, nothing else checks the feature flag.
- **Shared fixes.** Reveal in Finder and the Full Disk Access settings link move from `/usr/bin/open` to `NSWorkspace` in both editions.
- **Entitlements.** Lite has `app-sandbox`, `files.user-selected.read-write`, `files.bookmarks.app-scope` and `network.client`. Without `network.client` the WebKit view stays blank, even though the app makes no network requests; the privacy label is still Data Not Collected.
- **Build.** `pnpm desktop:build --lite` signs with Apple Development and applies the sandbox entitlements. `--lite --release` signs with Apple Distribution and produces a `.pkg`. The upload is a manual step in Transporter, and the build script never receives App Store Connect credentials. CI compiles and tests both editions on every PR.
- **Done means submitted.** The App Store Connect record, privacy label (Data Not Collected), description, screenshots and age rating are part of this effort.

## Sandbox spike results (2026-11-25)

A throwaway sandboxed build (branch `prototype/sandbox-reach`, signed with Apple Development) was run next to an unsandboxed Full edition build on the same Mac, scanning Macintosh HD (`/`) after allowing it through the Open dialog.

| Check | Result |
|---|---|
| 1. Reach of `/` | Same as the Full edition. Without Full Disk Access both measure 81.5 GiB / 991K entries; with it both measure 83.8 GiB / 1M entries. The only extra folder Lite cannot read is `~/Library/Keychains/<UUID>`. |
| 2. Full Disk Access | Widens Lite's reach: 663 → 277 unreadable folders (382 → 1 under Users). The existing silent probes report it correctly when they use the real home. |
| 3. Allowed location after relaunch | The security-scoped bookmark for `/` resolved (not stale) and `/` was scanned again without a new Open dialog. |
| 4. Rescan baseline | A rescan of an allowed folder ran incrementally from its Rescan baseline and picked up a new file. (`/` always scans in full in both editions.) |
| 5. Move to Trash | Worked inside an Allowed location; the item landed in `~/.Trash`. |
| 6. Quick Look and Reveal | Both worked (Reveal through `NSWorkspace`). |

Reach is not a reason to limit Lite, so no threshold is needed. Side finding: other apps, Terminal included, cannot read files in Lite's container without Full Disk Access, so any diagnostics Lite offers must be exported by the app itself (for example through a save dialog).

## Considered options

- **Sandbox the only app.** Rejected: it would lose the CLI and MCP and give less accurate startup-disk sizes for everyone.
- **Stay off the Mac App Store.** Rejected once the spike showed Lite reaches as much of the startup disk as the Full edition.
- **Hold features back in Lite to sell the Full edition.** Rejected: the Full edition is free and open source, and Apple rejects apps that look like demos.
