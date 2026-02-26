---
name: format
description: Run all formatting, linting and cleaning checks before committing code
---

Run all formatting, linting, and cleaning tasks that should be done before committing code. Fix any issues found automatically where possible.

## Steps

1. **Rust formatting** (requires nightly):
   ```bash
   cargo +nightly fmt --all
   ```

2. **TOML formatting**:
   ```bash
   taplo format --config .config/taplo.toml
   ```

3. **Zepter checks** (feature propagation):
   ```bash
   zepter run --config .config/zepter.yaml
   ```

4. **Clippy linting**:
   ```bash
   cargo clippy --all-targets --all-features --workspace -- -D warnings
   ```

## Notes

- Run formatting commands (steps 1-3) first as they may auto-fix issues
- Clippy warnings should be treated as errors (`-D warnings`)
- If `taplo` or `zepter` are not installed, inform the user how to install them:
  - `cargo install taplo-cli`
  - `cargo install zepter`
- Report all errors found and fix them where possible
