/**
 * Integration tests for the full upload → download round-trip.
 *
 * Run with: npx playwright test --project=integration
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --ipfs-server
 *   (node must be running on ws://localhost:10000 with IPFS enabled)
 *
 * These tests require:
 *   - A running local dev node
 *   - A browser wallet extension (or dev account injection)
 *   - Storage authorization for the test account
 */
import { test, expect } from "@playwright/test";

test.describe("Live Upload & Download", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await page.evaluate(() => {
      localStorage.setItem("bulletin-storage-type", "bulletin");
      localStorage.setItem("bulletin-network", "local");
    });
    await page.reload();
  });

  test.fixme(
    "upload text data and receive CID",
    async ({ page }) => {
      // TODO: Requires wallet connection and storage authorization.
      // Steps:
      // 1. Connect wallet with authorized dev account
      // 2. Navigate to /upload
      // 3. Enter text data
      // 4. Click upload
      // 5. Verify CID is returned
      // 6. Verify block number and index are displayed

      await page.goto("/upload");
      await expect(
        page.getByRole("heading", { name: "Upload Data" }),
      ).toBeVisible();
    },
  );

  test.fixme(
    "download previously uploaded data by CID",
    async ({ page }) => {
      // TODO: Requires a previously uploaded CID and IPFS gateway or P2P.
      // Steps:
      // 1. Navigate to /download
      // 2. Enter a known CID
      // 3. Connect via P2P or configure gateway
      // 4. Click Fetch Data
      // 5. Verify content matches original upload

      await page.goto("/download");
      await expect(
        page.getByRole("heading", { name: "Download Data" }),
      ).toBeVisible();
    },
  );
});
