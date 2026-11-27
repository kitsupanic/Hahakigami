# Local macOS builds

Use `pnpm desktop:build` when delivering a macOS `.app`. This helper selects an existing Apple Development identity, signs the bundle and verifies a stable designated requirement. Raw `pnpm tauri build --bundles app` can recreate a linker/ad-hoc identity and invalidate the user's macOS privacy grants. Use `--adhoc` only for an explicitly requested unsigned/ad-hoc build. Builds for other people use `pnpm desktop:build --release` (Developer ID, notarized, stapled; see ADR 0006). Never pass notarization credentials to a local build or fall back to an unnotarized release.

`pnpm desktop:build --lite` builds the sandboxed Hahakigami Lite edition (ADR 0007) the same way: Apple Development identity, and it fails unless the signature carries the App Sandbox entitlements. Lite builds with `--no-default-features --features lite`; never enable `automation` with `lite`. `--lite` cannot be combined with `--adhoc` or `--benchmark-output`.

`pnpm desktop:build --lite --release --provisioning-profile <path>` builds Lite for the Mac App Store: the app is signed with an existing Apple Distribution identity and the Lite entitlements plus `com.apple.application-identifier` and `com.apple.developer.team-identifier` (read from the profile with `security cms -D`, which only decodes it; the build fails unless the profile is for `<team>.app.hahakigami.lite` and the Apple Distribution identity's team), the sandbox and identifier entitlements are checked, the given Mac App Store `.provisionprofile` is embedded, and `productbuild` wraps the app in a `.pkg` signed with an existing Mac Installer Distribution identity (`--installer-identity` picks one by SHA-1 when there are several; `productbuild` gets its full name, and the build fails when two valid identities share it), then checks it with `pkgutil --check-signature`. It fails when either identity is missing and never falls back to another one. The profile is a file the person downloaded for `app.hahakigami.lite`; the script never creates or downloads one. The script refuses to run with App Store Connect, fastlane or notarization credentials in the environment (`APP_STORE_CONNECT_*`, `APPLE_API_*`, `APPLE_ID`, `APPLE_PASSWORD`, ...). Upload the `.pkg` (under `target/release/bundle/pkg/`) by hand with Transporter; never automate the upload or hand credentials to an agent.

The certificates, App ID, profile and App Store Connect record come from `scripts/appstore_lite_wizard.sh`, which a person runs. It records identifiers only, in the gitignored `.appstore-lite.env`. Agents don't run it.

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
