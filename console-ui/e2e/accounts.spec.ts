import { test, expect } from "./fixtures/test";

test.describe("Accounts Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/accounts");
  });

  test("shows page heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Accounts", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText("Manage your wallet connections and accounts"),
    ).toBeVisible();
  });

  test("displays wallet extensions card", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Wallet Extensions" }),
    ).toBeVisible();
    await expect(
      page.getByText("Connect a browser wallet to interact with the chain"),
    ).toBeVisible();
  });

  test("lists all four supported wallet extensions", async ({ page }) => {
    await expect(page.getByText("Polkadot.js")).toBeVisible();
    await expect(page.getByText("SubWallet")).toBeVisible();
    await expect(page.getByText("Talisman")).toBeVisible();
    await expect(page.getByText("Fearless")).toBeVisible();
  });

  test("shows not installed status for extensions in test environment", async ({
    page,
  }) => {
    // In Playwright browser, no extensions are installed
    const notInstalled = page.getByText("Not installed");
    await expect(notInstalled.first()).toBeVisible();
  });

  test("shows install links for undetected extensions", async ({ page }) => {
    const installButtons = page.getByRole("link", { name: "Install" });
    // All 4 extensions should have Install links since none are detected
    const count = await installButtons.count();
    expect(count).toBe(4);
  });

  test("install links open in new tab", async ({ page }) => {
    const installLink = page.getByRole("link", { name: "Install" }).first();
    await expect(installLink).toHaveAttribute("target", "_blank");
  });

  test("shows no extensions detected message", async ({ page }) => {
    await expect(
      page.getByText("No wallet extensions detected"),
    ).toBeVisible();
  });

  test("has refresh button for extension detection", async ({ page }) => {
    // Scope to main to avoid matching the Renew nav button's RefreshCw icon
    const main = page.locator("main");
    const refreshButton = main.locator("button:has(svg.lucide-refresh-cw)");
    await expect(refreshButton).toBeVisible();
  });

  test("does not show connected accounts card without wallet", async ({
    page,
  }) => {
    // "Connected Accounts" card only shows when a wallet is connected
    await expect(
      page.getByRole("heading", { name: "Connected Accounts" }),
    ).not.toBeVisible();
  });

  test("does not show selected account details without wallet", async ({
    page,
  }) => {
    await expect(
      page.getByRole("heading", { name: "Selected Account" }),
    ).not.toBeVisible();
  });
});
