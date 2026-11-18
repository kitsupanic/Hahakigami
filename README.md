# Hahakigami

A local disk-space analyzer for macOS 14+. Scan a disk or folder, explore it as a size-sorted tree, treemap or sunburst, and move a reviewed selection to the Trash.

Built with Rust, Tauri 2, Svelte 5 and TypeScript. No accounts, analytics or uploads.

## Features

- Scan the startup disk, any attached volume or a folder, or all local disks at once.
- Switch between **On disk** and **Logical** sizes. Hard links and full APFS clones are counted once.
- Search names, filter by file category and size, and break usage down by file type.
- Quick Look previews and Reveal in Finder.
- A cleanup queue that survives restarts. Nothing is deleted permanently: items go to the Trash after you review the exact paths.
- Fast rescans that reuse unchanged folders via macOS FSEvents.
- A command-line `scan` and a local MCP server for agents.

Scanning reads file metadata only, never file contents. Sizes do not predict how much space moving to Trash will free.

## Install

Download `Hahakigami-<version>-arm64.zip` from the [latest release](https://github.com/kitsupanic/Hahakigami/releases/latest), unzip it and move `Hahakigami.app` to Applications. Apple silicon only; on Intel, run from source.

The release is not notarized, so macOS blocks the first launch. Open the app once, then choose **Open Anyway** in **System Settings → Privacy & Security**, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Hahakigami.app
```

### Full Disk Access

Grant Full Disk Access under **System Settings → Privacy & Security → Full Disk Access** so macOS doesn't prompt for each protected folder. The sidebar shows its status. Each release has a new signing identity, so grant it again after updating. If prompts persist, remove the old entry and add the app again. See [ADR 0002](docs/adr/0002-detect-full-disk-access-with-silent-probes.md) for how the status is detected.

## Run from source

Requires Xcode command-line tools, stable Rust, Node.js 22.12+ and pnpm 11.

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

Quit the installed app first: only one instance can use the local data directory at a time.

To build a signed `.app` (uses an existing **Apple Development** identity from your keychain, so Full Disk Access survives rebuilds):

```sh
pnpm desktop:build
open target/release/bundle/macos/Hahakigami.app
```

With several identities, pass `python3 scripts/build_macos.py --identity <SHA1>`. With none, `--adhoc` builds unsigned, and macOS may ask for permissions again after every rebuild.

## CLI and MCP

The app's executable also runs scans without the UI:

```sh
app=/Applications/Hahakigami.app/Contents/MacOS/hahakigami
"$app" --help
"$app" scan /path/to/folder
```

`scan` prints one JSON result to stdout. Exit codes: `0` success, `1` failure, `2` bad arguments, `130` cancelled.

To browse results, add the MCP server to a client that supports local stdio servers:

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

Tools: `scan`, `status`, `cancel`, `summary`, `node`, `children`, `search`, `issues`. One scan per session; results end with the session. Command mode does not inherit the app's Full Disk Access grant, so check `issues` for denied locations.

## Architecture

- `crates/disk-scan`: the Tauri-independent metadata scanner and compact tree.
- `src-tauri`: scan sessions, queries, Finder/Trash actions and the CLI/MCP entry points.
- `src`: the Svelte UI, with Canvas maps laid out in workers.

Native paths stay in Rust; the UI sends scan and node IDs, never paths or shell strings. See [session lifecycle](docs/architecture/session-lifecycle.md), [ADRs](docs/adr/) and [GLOSSARY.md](GLOSSARY.md).

## Verify

```sh
cargo test --workspace
pnpm test
pnpm check
cargo clippy --workspace --all-targets -- -D warnings
```

[Verification notes](docs/VERIFICATION.md) cover packaged-app checks and known limits. [Benchmarks](docs/BENCHMARKS.md) compare against OpenDisk.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report security problems privately as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE). Inspired by [WinDirStat](https://windirstat.net/), [Radix](https://github.com/colinvkim/Radix) and [OpenDisk](https://github.com/137137137/OpenDisk).
