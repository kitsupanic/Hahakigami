# Detect Full Disk Access with silent probes, never prompting folders

macOS has no public API that reports whether an app has Full Disk Access. We used to avoid guessing: the user ticked a checkbox, we stored that acknowledgement against the executable's path, and every confirmation forced a restart. That meant asking again for each copy of the app (dev bundle vs `/Applications`), restarting even when access was already on, and a dialog that had to explain its own checkbox. We now detect it the way other disk analyzers do (Burrow, Radix, OpenDisk): open a few files that only Full Disk Access can read, read-only, read no bytes, and close them at once. The result is three states: on, off, unknown.

Opening a file gated only by Full Disk Access (such as `TCC.db`) fails silently when access is off, with no prompt. Opening a per-folder protected location (Documents, Desktop, Downloads, removable volumes) triggers the macOS consent prompt we exist to avoid, so those are never probed. We use no private APIs, read no TCC contents, and never change grants.

## Consequences

- The user and system `TCC.db` files are authoritative: only a refusal there (`EPERM`/`EACCES`) reports *off*. Other gated files can only confirm *on*. Anything else is *unknown*, and unknown never blocks.
- Scans are blocked while status is *off*, with a "Scan anyway" escape that warns about per-folder prompts.
- Probe behaviour is undocumented and changes between macOS versions (Radix uses a different set on macOS 27), so the probe set is a small per-version table with tests, and *unknown* is the safe default when it stops matching.
- macOS applies a new grant on next launch. After returning from System Settings we re-probe and offer a relaunch only if status is still *off*.
- The stored acknowledgement, its checkbox and the forced restart are removed.
