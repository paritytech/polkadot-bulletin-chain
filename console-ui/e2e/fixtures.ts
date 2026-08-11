// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

/**
 * Shared Playwright fixtures for Bulletin Chain integration tests.
 *
 * Provides a `localPage` fixture that automatically sets localStorage
 * for local dev network before any JS runs.
 */
import { test as base, expect, type Page } from "@playwright/test";

export interface TestOptions {
  /** Multiaddr of a collator serving bitswap; set per runtime in playwright.config.ts. */
  collatorMultiaddr: string;
}

/**
 * Custom fixture that configures localStorage for local dev network
 * and navigates to the app.
 */
export const test = base.extend<TestOptions & { localPage: Page }>({
  collatorMultiaddr: ["", { option: true }],
  localPage: async ({ page }, use) => {
    await page.addInitScript(() => {
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
