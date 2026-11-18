# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

App: macOS desktop app, Tauri 2 + Svelte 5, following native macOS conventions.

Website: https://hahakigami.com, a SvelteKit site (matches the app) kept in its own repository.

## Users

1. **Mac users with a full disk** (primary). They want to see what is taking space and clear it safely, without uploading anything or trusting an opaque "cleaner".
2. **Developers and agent users** (secondary). They care about the CLI, the local MCP server, the open-source code and the measured benchmarks.

## Purpose

Hahakigami is a local disk-space analyzer for macOS. It scans a disk or folder, shows storage as a size-sorted tree, treemap or sunburst, lets you inspect files and queue them for cleanup. The cleanup queue is grouped per disk; an item is trashed only after review and only when its volume's recorded UUID still matches.

## Position

- Fast: on a Home folder and on project folders, explorable results arrive ~30% sooner than OpenDisk, and it uses the least memory of the tools measured. It is still slower than OpenDisk on a full startup-disk (`/`) scan; claims must keep that caveat. Source: `docs/BENCHMARKS.md`.
- Honest sizes: verified sizes deduct shared APFS clone blocks; sizes are on disk only; partial coverage and warnings stay distinct. Sizes do not predict reclaimable space.
- Private: no accounts, analytics, remote content or uploads.
- Careful cleanup: nothing is deleted directly; files go to the Trash after the user reviews exact paths.
- Scriptable: the same executable runs headless scans as a CLI and a local stdio MCP server (`scan`, `status`, `cancel`, `list`, `summary`, `node`, `children`, `search`, `issues`).
- Open source, MIT license. Repo: https://github.com/kitsupanic/Hahakigami

## Constraints and status

- Version 0.1.0, described as a performance prototype. macOS 14+ only. Windows/Linux are future work.
- No public signed/notarized download yet. The site's main action is **Download**, shown as coming until the user provides a signed build.
- Full Disk Access is a macOS setting the user controls; the app only detects it (on/off/unknown). Never imply the app grants or bypasses it.

## Brand assets

- **The Guardian**: a broom spirit (hahakigami, the Japanese broom deity) with a dark face, straw-coloured bristles and a red knotted cord, on near-black. Source art: `src-tauri/icons/source.png`. Approved to front the website.
- App chrome follows the user's macOS accent colour (ADR 0001); that rule is for the app, not the website.

## Vocabulary

Use GLOSSARY.md terms, e.g. Disk scan, Scan all disks, Scan target, On disk, Scan warning, Scan budget, Full Disk Access status, Automation scan.

## Open decisions

- Pricing: not decided (do not state any).
- Download host and release date: not decided.
