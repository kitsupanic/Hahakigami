# Local macOS builds

Use `pnpm desktop:build` when delivering a macOS `.app`. This helper selects an existing Apple Development identity, signs the bundle and verifies a stable designated requirement. Raw `pnpm tauri build --bundles app` can recreate a linker/ad-hoc identity and invalidate the user's macOS privacy grants. Use `--adhoc` only for an explicitly requested unsigned/ad-hoc build. Builds for other people use `pnpm desktop:build --release` (Developer ID, notarized, stapled; see ADR 0006). Never pass notarization credentials to a local build or fall back to an unnotarized release.

`pnpm desktop:build --lite` builds the sandboxed Hahakigami Lite edition (ADR 0007) the same way: Apple Development identity, and it fails unless the signature carries the App Sandbox entitlements. Lite builds with `--no-default-features --features lite`; never enable `automation` with `lite`. `--lite` cannot be combined with `--adhoc`, `--benchmark-output` or (for now) `--release`.

Full Disk Access is detected only by the silent probes in ADR 0002 (`docs/adr/0002-detect-full-disk-access-with-silent-probes.md`). Never probe per-folder protected locations, use private TCC APIs, or change privacy grants. Do not export/import certificates, change keychain trust/ACLs, or weaken designated requirements to hide permission problems. Keep user app sessions and in-memory results intact when building updates.

## Project docs

### Issues

Specs and tickets live in this repository's GitHub Issues.

### Domain docs

Domain terms live in `GLOSSARY.md`; hard-to-reverse decisions are ADRs in `docs/adr/`.

### Scan sessions

Per-disk scan slots, invalidation and the rescan baseline: see `docs/architecture/session-lifecycle.md` before changing scan session state.

### Review

Code review applies the rules in `CODING_STANDARDS.md`.
