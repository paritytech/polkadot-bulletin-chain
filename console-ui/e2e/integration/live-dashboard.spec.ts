/**
 * Integration tests that require a running Bulletin Chain node.
 *
 * Run with: npx playwright test --project=integration
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev
 *   (node must be running on ws://localhost:10000)
 */
import { test, expect } from "@playwright/test";

test.describe("Live Dashboard", () => {
  test.beforeEach(async ({ page }) => {
    // Set localStorage to use local dev node
    await page.goto("/");
    await page.evaluate(() => {
      localStorage.setItem("bulletin-storage-type", "bulletin");
      localStorage.setItem("bulletin-network", "local");
    });
    await page.reload();
  });

  test("connects to local node and shows chain info", async ({ page }) => {
    // Wait for connection to establish
    await expect(page.getByText("connected")).toBeVisible({ timeout: 30_000 });

    // Chain info should be populated
    await expect(page.getByText("Chain Info")).toBeVisible();
    // Block number should appear (font-mono block number display)
    await expect(page.locator(".font-mono").first()).toBeVisible();
  });

  test("block number increments", async ({ page }) => {
    // Wait for connection
    await expect(page.getByText("connected")).toBeVisible({ timeout: 30_000 });

    // Get initial block number from the header badge
    const blockBadge = page.locator("header .font-mono");
    await expect(blockBadge).toBeVisible();
    const initialText = await blockBadge.textContent();

    // Wait for block number to change (blocks produced every ~6s)
    await expect(async () => {
      const currentText = await blockBadge.textContent();
      expect(currentText).not.toBe(initialText);
    }).toPass({ timeout: 15_000 });
  });
});
