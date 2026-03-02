/**
 * Custom Playwright test fixture that blocks WebSocket connections by default.
 *
 * All mocked E2E tests use this fixture. The app loads and renders normally
 * but stays in "connecting" state since no chain RPC responses are provided.
 * This is sufficient for testing UI structure, navigation, forms, and
 * interactions that don't depend on live chain data.
 *
 * For tests needing a full chain connection, use the `integration` project
 * with the base `test` from `@playwright/test`.
 */
import { test as base, expect } from "@playwright/test";
import { blockWebSockets } from "../mocks/rpc";

export const test = base.extend({
  page: async ({ page }, use) => {
    await blockWebSockets(page);
    await use(page);
  },
});

export { expect };
