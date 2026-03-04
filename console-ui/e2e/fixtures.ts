/**
 * Shared Playwright fixtures for Bulletin Chain integration tests.
 *
 * Provides a `localPage` fixture that automatically sets localStorage
 * for local dev network before any JS runs.
 */
import { test as base, expect, type Page } from "@playwright/test";

/**
 * Custom fixture that configures localStorage for local dev network
 * and navigates to the app.
 */
export const test = base.extend<{ localPage: Page }>({
  localPage: async ({ page }, use) => {
    await page.addInitScript(() => {
      localStorage.setItem("bulletin-storage-type", "bulletin");
      localStorage.setItem("bulletin-network", "local");
    });
    await page.goto("/");
    // Block number badge only appears when connected + block received
    await expect(page.getByTestId("block-number")).toBeVisible({
      timeout: 30_000,
    });
    await use(page);
  },
});

export { expect };
