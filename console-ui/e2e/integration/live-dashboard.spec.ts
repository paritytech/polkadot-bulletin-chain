/**
 * Integration tests that require a running Bulletin Chain node.
 *
 * Run with: playwright test
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --rpc-port 10000
 *   (node must be running on ws://localhost:10000)
 */
import { test, expect } from "../fixtures";

test.describe("Live Dashboard", () => {
  test("connects to local node and shows chain info", async ({ localPage: page }) => {
    // Chain info should be populated
    await expect(page.getByText("Chain Info")).toBeVisible();
    // Block number should appear
    await expect(page.getByTestId("block-number")).toBeVisible();
  });

  test("block number increments", async ({ localPage: page }) => {
    const blockBadge = page.getByTestId("block-number");
    const initialText = await blockBadge.textContent();

    // Wait for block number to change (blocks produced every ~6s)
    await expect(async () => {
      const currentText = await blockBadge.textContent();
      expect(currentText).not.toBe(initialText);
    }).toPass({ timeout: 15_000 });
  });
});
