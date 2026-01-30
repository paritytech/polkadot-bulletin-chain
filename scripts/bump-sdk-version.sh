#!/usr/bin/env bash
# Bump SDK version across all packages
# Usage: ./scripts/bump-sdk-version.sh <new_version>
# Example: ./scripts/bump-sdk-version.sh 0.2.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check arguments
if [ $# -ne 1 ]; then
    echo -e "${RED}❌ Error: Version argument required${NC}"
    echo "Usage: $0 <new_version>"
    echo "Example: $0 0.2.0"
    exit 1
fi

NEW_VERSION="$1"

# Validate semantic versioning format
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo -e "${RED}❌ Error: Invalid version format${NC}"
    echo "Version must follow semantic versioning: X.Y.Z or X.Y.Z-prerelease"
    echo "Examples: 0.1.0, 1.0.0, 0.2.0-alpha.1"
    exit 1
fi

echo -e "${GREEN}🔄 Bumping SDK version to ${NEW_VERSION}${NC}\n"

# Get current versions
RUST_CURRENT=$(grep '^version = ' "$ROOT_DIR/sdk/rust/Cargo.toml" | head -1 | cut -d '"' -f 2)
TS_CURRENT=$(grep '"version":' "$ROOT_DIR/sdk/typescript/package.json" | head -1 | cut -d '"' -f 4)

echo "Current versions:"
echo "  Rust:       $RUST_CURRENT"
echo "  TypeScript: $TS_CURRENT"
echo ""
echo "New version: $NEW_VERSION"
echo ""

# Confirm
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 1
fi

# Update Rust SDK
echo -e "\n${YELLOW}📦 Updating Rust SDK...${NC}"
sed -i.bak "s/^version = \"$RUST_CURRENT\"/version = \"$NEW_VERSION\"/" "$ROOT_DIR/sdk/rust/Cargo.toml"
rm "$ROOT_DIR/sdk/rust/Cargo.toml.bak"
echo -e "${GREEN}✅ Updated sdk/rust/Cargo.toml${NC}"

# Update TypeScript SDK
echo -e "\n${YELLOW}📦 Updating TypeScript SDK...${NC}"
sed -i.bak "s/\"version\": \"$TS_CURRENT\"/\"version\": \"$NEW_VERSION\"/" "$ROOT_DIR/sdk/typescript/package.json"
rm "$ROOT_DIR/sdk/typescript/package.json.bak"
echo -e "${GREEN}✅ Updated sdk/typescript/package.json${NC}"

# Update package-lock.json if it exists
if [ -f "$ROOT_DIR/sdk/typescript/package-lock.json" ]; then
    echo -e "\n${YELLOW}📦 Updating package-lock.json...${NC}"
    cd "$ROOT_DIR/sdk/typescript"
    npm install --package-lock-only
    cd "$ROOT_DIR"
    echo -e "${GREEN}✅ Updated sdk/typescript/package-lock.json${NC}"
fi

# Summary
echo -e "\n${GREEN}✅ Version bump complete!${NC}\n"
echo "Next steps:"
echo "  1. Review changes: git diff"
echo "  2. Run tests:"
echo "     cd sdk/rust && cargo test"
echo "     cd sdk/typescript && npm test"
echo "  3. Commit changes:"
echo "     git add sdk/rust/Cargo.toml sdk/typescript/package.json"
echo "     git commit -m \"chore(sdk): bump version to $NEW_VERSION\""
echo "  4. Create tag:"
echo "     git tag sdk-v$NEW_VERSION"
echo "  5. Push:"
echo "     git push origin main --tags"
echo ""
echo -e "${YELLOW}📚 See RELEASING.md for full release process${NC}"
