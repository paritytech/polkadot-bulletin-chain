/**
 * Integration tests that require a running Bulletin Chain node.
 *
 * Run with: npx playwright test --project=integration
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --rpc-port 10000
 *   (node must be running on ws://localhost:10000)
 */
import { test, expect } from "@playwright/test";

test.describe("Live Dashboard", () => {
  test.beforeEach(async ({ page }) => {
    // Set localStorage before JS runs so the app starts with "local" network
    await page.addInitScript(() => {
      localStorage.setItem("bulletin-storage-type", "bulletin");
      localStorage.setItem("bulletin-network", "local");
    });
    await page.goto("/");
  });

  test("connects to local node and shows chain info", async ({ page }) => {
    // Wait for connection — block number badge only shows when connected
    await expect(page.locator("header .font-mono")).toBeVisible({
      timeout: 30_000,
    });

    // Chain info should be populated
    await expect(page.getByText("Chain Info")).toBeVisible();
    // Block number should appear (font-mono block number display)
    await expect(page.locator(".font-mono").first()).toBeVisible();
  });

  test("block number increments", async ({ page }) => {
    // Wait for connection
    const blockBadge = page.locator("header .font-mono");
    await expect(blockBadge).toBeVisible({ timeout: 30_000 });
    const initialText = await blockBadge.textContent();

    // Wait for block number to change (blocks produced every ~6s)
    await expect(async () => {
      const currentText = await blockBadge.textContent();
      expect(currentText).not.toBe(initialText);
    }).toPass({ timeout: 15_000 });
  });
});
