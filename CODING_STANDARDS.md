# Coding standards

The reviewer agent applies these to diffs. Flag any violation.

## Commands read one source of state
A menu or keyboard command's handler and its `can.*` availability check must read the same state from createApp (`app.*`). Never use a view-local derived copy in one of them, such as App.svelte's `summary` (from `app.shown`), which is null while Home is shown. A mismatch leaves a command enabled but inert, or disabled but wired.

## `can.*` is the whole availability answer
A menu command's `enabled` and its button's `disabled` map `app.can.<command>` straight through. Put any extra condition (selection state, an add in flight, a preference) into the `can` getter in createApp, not into an App.svelte expression around it.

## Hidden results are read-only
While Home or the cleanup queue is shown, the explorer's results are loaded but hidden. Commands must not change their selection, folder or history then. Gate such commands on the visible view (`exploring`, `!homeOpen`, and so on).

## New menu commands update the Home allowlist
When adding a command id to `commandIds`, update `enabledOnHome` in src/App.menu.test.ts on purpose. Add the id only if the command is fine on the welcome page (global, or visibly acting on the scan). Otherwise gate it on `!homeOpen`. Do not paste the id in just to make the test pass.

## The scan run gets its Rescan baseline through baseline.rs
`run_scan` asks `baseline.rs` for the baseline before the scan (`acquire`) and hands the ticket back after it (`publish`). Nothing else locks the rescan cache, calls `cache::` or `events::`, or compares journal epochs and cursors. See "The baseline module" in docs/architecture/session-lifecycle.md.

## Commands get scans through slots
A Tauri command gets a scan from `slots.rs`: `find` for reads, `hold` before changing state or touching files, `completed_snapshot` when it needs a completed scan. Never lock a slot, read or store a session's `valid`, or check `finishing` and `Completed` by hand in a command. See "The slots module" in docs/architecture/session-lifecycle.md for the lock order.
