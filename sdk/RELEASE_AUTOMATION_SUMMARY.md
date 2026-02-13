# SDK Release Automation

Automated release pipeline for publishing both SDKs to crates.io, npm, and GitHub Releases.

## How to Release

```bash
# 1. Bump version in both packages
./scripts/bump-sdk-version.sh 0.2.0

# 2. Test locally
cd sdk/rust && cargo test --all-features
cd sdk/typescript && npm test

# 3. Commit, tag, and push
git add sdk/rust/Cargo.toml sdk/typescript/package.json sdk/typescript/package-lock.json
git commit -m "chore(sdk): bump version to 0.2.0"
git push origin main
git tag sdk-v0.2.0 && git push origin sdk-v0.2.0

# GitHub Actions handles the rest (build, validate, publish).
```

You can also trigger manually: Actions > Release SDK > Run workflow.

## Required Secrets

Set in GitHub repo settings (Settings > Secrets and variables > Actions):

- `CARGO_REGISTRY_TOKEN` - from https://crates.io/me > API Tokens
- `NPM_TOKEN` - from npm or https://www.npmjs.com/ > Access Tokens

## Version Strategy

Semantic versioning: `Major.Minor.Patch`

Pre-release tags (e.g. `sdk-v0.2.0-alpha.1`) create a GitHub Release but skip crates.io/npm publishing.

## Rollback

Don't unpublish. Publish a fixed patch version and deprecate the bad one:

```bash
cargo yank --vers 0.2.0 bulletin-sdk-rust
npm deprecate @bulletin/sdk@0.2.0 "Use 0.2.1"
```
