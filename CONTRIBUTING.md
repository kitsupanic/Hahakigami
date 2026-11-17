# Contributing

Thanks for helping with Hahakigami. Bug reports, fixes and focused features are welcome.

## Before you start

- Open an issue first for anything larger than a small fix, so we can agree on the approach.
- Hahakigami runs on macOS 14+ only. See [Run from source](README.md#run-from-source) for setup.
- Read [GLOSSARY.md](GLOSSARY.md) for the project's terms and [docs/adr/](docs/adr/) for past decisions.

## Making a change

1. Branch from `main`.
2. Keep the change small and add or update tests for the behaviour you touch.
3. Follow [CODING_STANDARDS.md](CODING_STANDARDS.md).
4. Run the checks CI runs:

   ```sh
   pnpm check
   pnpm test
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   python3 -m unittest discover -s scripts -p 'test_*.py'
   ```

5. Open a pull request that says what changed, why, and how you checked it. Add a screenshot for UI changes.

## Safety rules

Hahakigami moves files to Trash and reads protected folders, so a few rules are firm:

- Never add a permanent-delete path. Cleanup only moves items to Trash.
- Full Disk Access is detected only by the silent probes in [ADR 0002](docs/adr/0002-detect-full-disk-access-with-silent-probes.md). Do not probe protected folders one by one or use private TCC APIs.
- Do not weaken code-signing requirements to hide permission problems.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
