# Sign releases with Developer ID and notarize them

Builds for other people are signed with a **Developer ID Application** identity, notarized and stapled, and shared as a DMG. Local builds keep the **Apple Development** identity. Both go through `scripts/build_macos.py` (`pnpm desktop:build`, and `pnpm desktop:build --release` for releases), so every bundle gets the same checks: sealed, the configured identifier, and a designated requirement tied to the team, not to one executable hash.

Apple Development builds only run on Macs registered to the team, and an unnotarized download is blocked by Gatekeeper. Developer ID is the only identity that runs on any Mac. Its designated requirement names the team (`certificate leaf[subject.OU]`), so Full Disk Access granted to one release carries over to the next. Ad-hoc releases lost that grant on every update.

## Consequences

- `--release` accepts only an existing Developer ID Application identity in the keychain. Like the local build, it never imports a certificate, exports a key or changes keychain trust; `APPLE_CERTIFICATE*` variables are always dropped.
- A release fails unless one complete set of notarization credentials is in the environment (an App Store Connect API key, or Apple ID + app-specific password + team ID), so a build is never shared unnotarized by accident.
- The release bundle must be signed with the hardened runtime, pass `stapler validate` and be accepted by `spctl`.
- Local and release builds have different leaf certificates, so macOS treats them as different apps for privacy grants. Full Disk Access is granted separately to each.
- Moving from a Personal Team to the paid team changes the team ID once; grants made under the old team do not carry over.
