# Contributing to the Polkadot Bulletin Chain

Thank you for your interest in contributing! All contributions are made via pull requests that require maintainer approval before merging.

## Getting Started

1. Install the [Polkadot SDK dependencies](https://docs.polkadot.com/develop/parachains/install-polkadot-sdk)
2. Fork the repository and clone your fork
3. Build and run tests:

```bash
cargo build --release
cargo test
```

## Rules

1. **No `--force` pushes** or rewriting of shared branch history.
2. **All modifications** must be made via a **pull request** to solicit feedback.
3. A pull request **must not be merged until CI has finished successfully**.
4. All review comments must be addressed before merging.

## Pull Request Process

1. Create a feature branch from `main` (e.g. `your-name/my-feature`).
2. Make your changes, ensuring tests pass locally.
3. Submit your PR as "Draft" while still in progress. Mark it "Ready for review" when complete.
4. Include a clear description of what the PR does and why.

## Code Style

- Run formatting and linting checks before submitting: `cargo +nightly fmt --all`, `taplo format`, `cargo clippy --all-targets --all-features --workspace -- -D warnings`.
- Follow existing patterns in the codebase.

## Licensing

Contributions are accepted under the project's existing licenses:

| Component | License |
|---|---|
| Runtimes, applications (`runtimes/`, `console-ui/`) | GPL-3.0-only |
| Pallets, SDKs, libraries, tools (`pallets/`, `sdk/`, `examples/`, `scripts/`) | Apache-2.0 |

By submitting a pull request, you agree that your contributions will be licensed under the applicable license for the component you are modifying.

## Security

If you discover a security vulnerability, **do not open a public issue**. See Parity's security disclosure process at https://parity.io/bug-bounty.
