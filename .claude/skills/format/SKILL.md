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

5. **TypeScript formatting and linting** (Biome):
   Find all directories containing a `biome.json` or `biome.jsonc` config and run Biome in each:
   ```bash
   find . -name 'biome.json' -o -name 'biome.jsonc' | while read config; do
     dir=$(dirname "$config")
     echo "Running Biome in $dir"
     (cd "$dir" && npx @biomejs/biome check --write .)
   done
   ```

## Notes

- Run formatting commands (steps 1-3, 5) first as they may auto-fix issues
- Clippy warnings should be treated as errors (`-D warnings`)
- If `taplo` or `zepter` are not installed, inform the user how to install them:
  - `cargo install taplo-cli`
  - `cargo install zepter`
- If nightly fmt is not installed help user install with `rustup component add rustfmt --toolchain nightly`
- Biome handles TypeScript/JavaScript formatting, linting, and import sorting. It uses the `biome.json` config in each directory for project-specific rules.
- If `npx @biomejs/biome` is not available, ensure the project has `@biomejs/biome` as a devDependency (`npm install --save-dev @biomejs/biome`)
- Report all errors found and fix them where possible
