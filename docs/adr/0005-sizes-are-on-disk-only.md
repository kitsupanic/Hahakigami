# Sizes are on disk only

The app used to carry two sizes for every file and folder: the allocated blocks it occupies (**On disk**) and the byte length of its content (a **Logical size**), with a switch in the UI to pick one. Logical size is now gone as a product concept. Every size the app records, shows, sorts by or exports is on disk.

## Why

- **One size is enough.** On disk is the number people act on. It already deducts hard links and verified APFS clone blocks, so it is closest to what a cleanup could free, though it is still not a prediction (see **On disk** in `GLOSSARY.md`). Logical size answered a different question that the app never helped anyone act on, and two sizes next to each other made people wonder which one to trust.
- **Less surface.** The second size meant a UI switch, a second column and sort order in every list, a second roll-up through the scanner, the incremental rescan and clone accounting, a second field in the rescan cache, and a second option in the CLI and MCP. Dropping it removes all of that and the tests that kept the two in step.

## What stays: a private Length

`crates/disk-scan` still keeps one byte count per file, the **Length** (`Node.length`, `CloneMetadata.data_length`, `length` on bulk records and in diagnostic JSON). It exists for two jobs only:

- verifying clones: clone accounting compares lengths before it deducts shared blocks;
- detecting changed files on rescan: the cached Length is part of what says a file is unchanged.

It is stored per file in the rescan cache. It is never summed per folder, never shown and never exposed through the CLI or MCP.

## Consequences

- The rescan cache magic is `HAHCAC04` (was `HAHCAC03`). Older caches are rejected and each disk gets one full rescan.
- The CLI and MCP output no longer has `logicalBytes`, and the `"logical"` size and sort option is gone. A request that still passes it gets the normal unknown-value error. MCP tool descriptions mention allocated size only.
- Scripts and benchmark docs compare against other apps on on-disk size only.
