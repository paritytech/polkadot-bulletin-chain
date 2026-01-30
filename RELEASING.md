# Releasing the Bulletin SDK

This document describes the release process for the Bulletin SDK packages (Rust and TypeScript).

## Overview

The SDK uses semantic versioning and releases are published to:
- **Rust SDK**: [crates.io](https://crates.io/crates/bulletin-sdk-rust)
- **TypeScript SDK**: [npm](https://www.npmjs.com/package/@bulletin/sdk)
- **GitHub Releases**: [Releases page](https://github.com/paritytech/polkadot-bulletin-chain/releases)

## Prerequisites

### Required Credentials

#### For Maintainers

You need the following secrets configured in GitHub Actions:

1. **`CARGO_REGISTRY_TOKEN`**: Token for publishing to crates.io
   - Get from: https://crates.io/me
   - Settings → API Tokens → Create Token
   - Add as repository secret in GitHub

2. **`NPM_TOKEN`**: Token for publishing to npm
   - Generate: `npm login` then `npm token create`
   - Or via npmjs.com → Account Settings → Access Tokens
   - Add as repository secret in GitHub

### Local Development

For local testing:
```bash
# Verify you have access
cargo login
npm login
```

## Release Process

### 1. Prepare the Release

#### Update Version Numbers

Version numbers must be synchronized across all packages:

**Rust SDK** (`sdk/rust/Cargo.toml`):
```toml
[package]
version = "0.2.0"  # Update this
```

**TypeScript SDK** (`sdk/typescript/package.json`):
```json
{
  "version": "0.2.0"  # Update this (must match Rust version)
}
```

#### Update CHANGELOG

Update `CHANGELOG.md` in both SDK directories with the new version:

```markdown
## [0.2.0] - 2024-01-29

### Added
- New feature X
- New feature Y

### Changed
- Breaking change Z

### Fixed
- Bug fix A
```

#### Run Tests

Ensure all tests pass:

```bash
# Rust SDK
cd sdk/rust
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check

# TypeScript SDK
cd sdk/typescript
npm test
npm run typecheck
npm run lint
```

### 2. Commit and Tag

```bash
# Commit version bumps
git add sdk/rust/Cargo.toml sdk/typescript/package.json CHANGELOG.md
git commit -m "chore(sdk): bump version to 0.2.0"

# Create and push tag
git tag sdk-v0.2.0
git push origin main --tags
```

**Tag Format**: `sdk-v{major}.{minor}.{patch}`
- ✅ `sdk-v0.1.0`
- ✅ `sdk-v1.0.0`
- ✅ `sdk-v1.2.3`
- ❌ `v0.1.0` (missing `sdk-` prefix)
- ❌ `0.1.0` (missing `sdk-v` prefix)

### 3. Automated Release

Once you push the tag, GitHub Actions will automatically:

1. ✅ **Validate** versions match across packages
2. ✅ **Build** both Rust and TypeScript SDKs
3. ✅ **Test** all functionality
4. ✅ **Publish** to crates.io
5. ✅ **Publish** to npm
6. ✅ **Create** GitHub Release with notes

Monitor the workflow: https://github.com/paritytech/polkadot-bulletin-chain/actions

### 4. Verify Release

After the workflow completes:

#### Check crates.io
```bash
# Check version is published
cargo search bulletin-sdk-rust

# Try installing
cargo install bulletin-sdk-rust --version 0.2.0
```

Visit: https://crates.io/crates/bulletin-sdk-rust

#### Check npm
```bash
# Check version is published
npm view @bulletin/sdk versions

# Try installing
npm install @bulletin/sdk@0.2.0
```

Visit: https://www.npmjs.com/package/@bulletin/sdk

#### Check GitHub Release

Visit: https://github.com/paritytech/polkadot-bulletin-chain/releases

Verify:
- ✅ Release notes are complete
- ✅ Tarball is attached
- ✅ Links to crates.io and npm are correct

## Dry Run (Testing)

To test the release process without actually publishing:

```bash
# Via GitHub Actions UI
# Go to Actions → Release SDK → Run workflow
# Set:
#   - version: 0.2.0
#   - dry_run: true
```

This will:
- ✅ Build and test both SDKs
- ✅ Validate versions
- ✅ Package everything
- ❌ Skip actual publishing
- ❌ Skip creating GitHub Release

## Manual Release (Fallback)

If automated release fails, you can publish manually:

### Rust SDK

```bash
cd sdk/rust

# Dry run first
cargo publish --dry-run

# Actual publish
cargo publish
```

### TypeScript SDK

```bash
cd sdk/typescript

# Build
npm run build

# Verify package contents
npm pack
tar -xzf bulletin-sdk-0.2.0.tgz
ls package/

# Publish (with 2FA if enabled)
npm publish --access public
```

### GitHub Release

```bash
# Install gh CLI if needed: brew install gh

# Create release
gh release create sdk-v0.2.0 \
  --title "Bulletin SDK v0.2.0" \
  --notes-file RELEASE_NOTES.md \
  sdk/typescript/bulletin-sdk-0.2.0.tgz
```

## Version Strategy

### Semantic Versioning

The SDK follows [SemVer 2.0.0](https://semver.org/):

- **Major (X.0.0)**: Breaking changes
  - API changes that break existing code
  - Removed features
  - Changed behavior

- **Minor (0.X.0)**: New features (backwards compatible)
  - New methods/functions
  - New optional parameters
  - Performance improvements
  - Deprecations (not removals)

- **Patch (0.0.X)**: Bug fixes (backwards compatible)
  - Bug fixes
  - Documentation updates
  - Internal refactoring

### Pre-releases

For alpha/beta releases:

```bash
# Alpha
git tag sdk-v0.2.0-alpha.1

# Beta
git tag sdk-v0.2.0-beta.1

# Release candidate
git tag sdk-v0.2.0-rc.1
```

Pre-release tags will create a GitHub Release but won't publish to crates.io or npm (workflow skips publish jobs for pre-release tags).

## Troubleshooting

### "Version already published"

If a version is already published to crates.io or npm:
- You cannot unpublish (unless within 72 hours on crates.io)
- Bump to next patch version
- Re-tag and release

### "Authentication failed"

Check tokens are valid:
```bash
# crates.io
cargo login

# npm
npm whoami
npm token list
```

Regenerate if expired and update GitHub secrets.

### "Tests failed in CI"

- Fix the failing tests
- Commit fixes
- Delete and recreate the tag:
  ```bash
  git tag -d sdk-v0.2.0
  git push origin :refs/tags/sdk-v0.2.0
  # Fix issues, commit
  git tag sdk-v0.2.0
  git push origin sdk-v0.2.0
  ```

### "Version mismatch error"

The workflow validates that Rust and TypeScript versions match. If they don't:
```
❌ Version mismatch!
   Cargo.toml: 0.2.0
   package.json: 0.1.0
```

Fix by updating both to the same version.

## Rollback

To rollback a release:

1. **DO NOT** unpublish from crates.io or npm (breaks dependents)
2. Instead, publish a new patch version with fixes
3. Mark the bad release as "yanked" on crates.io:
   ```bash
   cargo yank --vers 0.2.0 bulletin-sdk-rust
   ```
4. Deprecate on npm:
   ```bash
   npm deprecate @bulletin/sdk@0.2.0 "This version has critical issues. Please upgrade to 0.2.1"
   ```

## Post-Release Checklist

After a successful release:

- [ ] Verify packages are published and installable
- [ ] Update documentation if API changed
- [ ] Announce release (GitHub Discussions, Discord, etc.)
- [ ] Close related issues/PRs
- [ ] Update examples if needed
- [ ] Start next version in `develop` branch

## Release Cadence

**Recommended schedule:**

- **Patch releases**: As needed for bug fixes (within days)
- **Minor releases**: Every 2-4 weeks for new features
- **Major releases**: Every 3-6 months for breaking changes

Coordinate major releases with:
- Polkadot SDK releases
- Bulletin Chain runtime upgrades
- Breaking changes in dependencies

## Support

For questions about releasing:
- Open an issue: https://github.com/paritytech/polkadot-bulletin-chain/issues
- Ask in discussions: https://github.com/paritytech/polkadot-bulletin-chain/discussions
