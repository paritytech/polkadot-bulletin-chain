/**
 * Shared helpers for Bulletin Chain integration tests.
 */
import { expect, type Page } from "@playwright/test";

/** Wait for chain connection after SPA navigation (block number in header). */
export async function waitForConnection(page: Page) {
  await expect(page.getByTestId("block-number")).toBeVisible({
    timeout: 30_000,
  });
}

/**
 * Wait for the chain to produce enough blocks so that mortal transactions
 * have a valid era. On a freshly started --dev chain, submitting at block 1
 * can produce "Stale" errors because the mortality checkpoint is too recent.
 */
export async function waitForMinBlock(page: Page, minBlock = 3) {
  await expect(async () => {
    const text = await page.getByTestId("block-number").textContent();
    const num = parseInt(text?.replace(/[#,]/g, "") ?? "0", 10);
    expect(num).toBeGreaterThanOrEqual(minBlock);
  }).toPass({ timeout: 30_000 });
}

/** Click a nav link in the header. Uses exact match to avoid Dashboard quick-action links. */
export async function navigateTo(page: Page, name: string) {
  await page.locator("nav").getByRole("link", { name, exact: true }).click();
}
