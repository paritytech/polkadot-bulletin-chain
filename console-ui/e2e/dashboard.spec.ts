import { test, expect } from "./fixtures/test";

test.describe("Dashboard", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("shows dashboard heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Dashboard", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText("Overview of your Bulletin Chain activity"),
    ).toBeVisible();
  });

  test("displays welcome card", async ({ page }) => {
    await expect(
      page.getByText("Welcome to Bulletin Chain Console"),
    ).toBeVisible();
    await expect(
      page.getByText(/Store and retrieve data on the Polkadot Bulletin Chain/),
    ).toBeVisible();
  });

  test("displays welcome card feature highlights", async ({ page }) => {
    await expect(page.getByText("Decentralized Storage")).toBeVisible();
    await expect(page.getByText("IPFS Compatible")).toBeVisible();
    await expect(page.getByText("Authorization Based")).toBeVisible();
  });

  test("displays Chain Info card with network name", async ({ page }) => {
    await expect(page.getByText("Chain Info")).toBeVisible();
    await expect(page.getByText("Current network status")).toBeVisible();
    // Network name should be visible in the Chain Info card (default: Bulletin Paseo)
    await expect(
      page.getByRole("main").getByText(/Bulletin Paseo|Local Dev/),
    ).toBeVisible();
  });

  test("displays Quick Actions card with all links", async ({ page }) => {
    await expect(page.getByText("Quick Actions")).toBeVisible();
    await expect(page.getByRole("link", { name: /Upload Data/ })).toBeVisible();
    await expect(
      page.getByRole("link", { name: /Download by CID/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: /Explore Blocks/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: /Renew Storage/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: /View Authorizations/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: /Storage Faucet/ }),
    ).toBeVisible();
  });

  test("displays Account card with Connect Wallet", async ({ page }) => {
    await expect(
      page.getByText("Connect a wallet to get started"),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: /Connect Wallet/ }),
    ).toBeVisible();
  });

  test("quick action links navigate to correct pages", async ({ page }) => {
    // Click "Explore Blocks" quick action
    await page.getByRole("link", { name: /Explore Blocks/ }).click();
    await expect(
      page.getByRole("heading", { name: /Explorer/i, level: 1 }),
    ).toBeVisible();
  });

  test("shows Web3 Storage welcome card when in web3storage mode", async ({
    page,
  }) => {
    // Switch to Web3 Storage mode
    const storageTypeSelector = page
      .locator("header")
      .getByRole("combobox")
      .nth(1);
    await storageTypeSelector.click();
    await page.getByRole("option", { name: /Web3 Storage/i }).click();

    // Should show the web3storage welcome card
    await expect(
      page.getByText("Welcome to Web3 Storage Console"),
    ).toBeVisible();
    await expect(
      page.getByText("Overview of your Web3 Storage activity"),
    ).toBeVisible();
  });
});
