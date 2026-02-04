# SDK Release Checklist

Quick reference for releasing the Bulletin SDK packages.

## Release Automation

The release process is fully automated via GitHub Actions (`.github/workflows/release-sdk.yml`):

**Trigger**: Push a tag matching `sdk-v*.*.*` (e.g., `sdk-v0.1.0`)

**Automated steps**:
1. Validates versions in Cargo.toml and package.json match the tag
2. Builds and tests both Rust and TypeScript SDKs
3. Publishes to crates.io (Rust) and npm (TypeScript)
4. Creates GitHub Release with auto-generated notes

**Required secrets** (configured in GitHub):
- `CARGO_REGISTRY_TOKEN` - For crates.io publishing
- `NPM_TOKEN` - For npm publishing

## Pre-Release

- [ ] All tests passing locally
- [ ] CI is green on main branch
- [ ] CHANGELOG.md updated with changes
- [ ] Documentation updated if API changed
- [ ] No known critical bugs

## Version Bump

```bash
# Use the bump script
./scripts/bump-sdk-version.sh 0.2.0

# Or manually update:
# - sdk/rust/Cargo.toml
# - sdk/typescript/package.json
```

## Testing

```bash
# Rust SDK
cd sdk/rust
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo build --release
cargo build --release --no-default-features
cargo package --allow-dirty  # Verify package contents

# TypeScript SDK
cd sdk/typescript
npm test
npm run typecheck
npm run lint
npm run build
npm pack  # Verify package contents
tar -tzf bulletin-sdk-*.tgz
```

## Commit & Tag

```bash
# Commit version bump
git add sdk/rust/Cargo.toml sdk/typescript/package.json CHANGELOG.md
git commit -m "chore(sdk): bump version to 0.2.0"

# Push to main
git push origin main

# Create and push tag
git tag sdk-v0.2.0
git push origin sdk-v0.2.0
```

## Monitor Release

1. Go to https://github.com/paritytech/polkadot-bulletin-chain/actions
2. Watch "Release SDK" workflow
3. Monitor each step:
   - [ ] Version validation
   - [ ] Rust SDK build
   - [ ] TypeScript SDK build
   - [ ] crates.io publish
   - [ ] npm publish
   - [ ] GitHub Release creation

## Verify Release

### crates.io

```bash
# Wait ~5 minutes for crates.io to index
cargo search bulletin-sdk-rust
cargo install bulletin-sdk-rust --version 0.2.0
```

Check: https://crates.io/crates/bulletin-sdk-rust

### npm

```bash
# Wait ~2 minutes for npm to propagate
npm view @bulletin/sdk versions
npm install @bulletin/sdk@0.2.0
```

Check: https://www.npmjs.com/package/@bulletin/sdk

### GitHub Release

Check: https://github.com/paritytech/polkadot-bulletin-chain/releases

Verify:
- [ ] Release notes are complete
- [ ] Tarball is attached
- [ ] Links work
- [ ] Version tag is correct

## Post-Release

- [ ] Test installation in fresh project
- [ ] Update examples if needed
- [ ] Announce release
  - [ ] GitHub Discussions
  - [ ] Discord/Community channels
- [ ] Close related issues
- [ ] Update project documentation

## Dry Run (Testing Only)

To test without actually publishing:

1. Go to Actions → Release SDK → Run workflow
2. Set:
   - version: `0.2.0`
   - dry_run: `true`
3. Monitor workflow

This validates everything without publishing.

## Rollback

If something goes wrong:

1. **DO NOT** unpublish packages (breaks downstream users)
2. Publish a fixed version:
   ```bash
   ./scripts/bump-sdk-version.sh 0.2.1
   git add . && git commit -m "fix(sdk): critical bug fix"
   git push origin main
   git tag sdk-v0.2.1 && git push origin sdk-v0.2.1
   ```
3. Deprecate bad version:
   ```bash
   # crates.io
   cargo yank --vers 0.2.0 bulletin-sdk-rust

   # npm
   npm deprecate @bulletin/sdk@0.2.0 "Critical bug, please upgrade to 0.2.1"
   ```

## Common Issues

### "Version already exists"
- Bump to next patch version
- Cannot reuse version numbers

### "Tests failed"
- Fix issues locally
- Delete tag: `git tag -d sdk-v0.2.0 && git push origin :refs/tags/sdk-v0.2.0`
- Fix, commit, retag

### "Authentication failed"
- Check GitHub secrets: `CARGO_REGISTRY_TOKEN`, `NPM_TOKEN`
- Regenerate tokens if expired

### "Version mismatch"
- Ensure Rust and TypeScript versions match exactly
- Run `./scripts/bump-sdk-version.sh` again

## Quick Commands

```bash
# Check current versions
grep '^version = ' sdk/rust/Cargo.toml
grep '"version":' sdk/typescript/package.json

# Test release locally
cd sdk/rust && cargo publish --dry-run
cd sdk/typescript && npm publish --dry-run

# View recent releases
gh release list --limit 5

# View workflow runs
gh run list --workflow=release-sdk.yml --limit 5
```

## Need Help?

- Full guide: [RELEASING.md](../RELEASING.md)
- Open issue: https://github.com/paritytech/polkadot-bulletin-chain/issues
- Ask in discussions: https://github.com/paritytech/polkadot-bulletin-chain/discussions
