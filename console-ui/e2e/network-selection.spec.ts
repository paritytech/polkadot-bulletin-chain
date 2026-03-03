import { test, expect } from "./fixtures/test";

test.describe("Network Selection", () => {
  test("network switcher shows available networks", async ({ page }) => {
    await page.goto("/");

    // Open the network selector dropdown
    const networkSwitcher = page
      .locator("header")
      .getByRole("combobox")
      .first();
    await networkSwitcher.click();

    // Check that bulletin networks are listed
    await expect(page.getByRole("option", { name: /Local Dev/i })).toBeVisible();
    await expect(
      page.getByRole("option", { name: /Bulletin Westend/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("option", { name: /Bulletin Paseo/i }),
    ).toBeVisible();
  });

  test("persists network selection in localStorage", async ({ page }) => {
    await page.goto("/");

    // Open network selector and pick "Local Dev"
    const networkSwitcher = page
      .locator("header")
      .getByRole("combobox")
      .first();
    await networkSwitcher.click();
    await page.getByRole("option", { name: /Local Dev/i }).click();

    // Verify localStorage was updated
    const storedNetwork = await page.evaluate(() =>
      localStorage.getItem("bulletin-network"),
    );
    expect(storedNetwork).toBe("local");
  });

  test("restores network selection from localStorage on reload", async ({
    page,
  }) => {
    await page.goto("/");

    // Switch to Westend via the UI
    const networkSwitcher = page
      .locator("header")
      .getByRole("combobox")
      .first();
    await networkSwitcher.click();
    await page.getByRole("option", { name: /Bulletin Westend/i }).click();

    // Verify it was saved
    const storedNetwork = await page.evaluate(() =>
      localStorage.getItem("bulletin-network"),
    );
    expect(storedNetwork).toBe("westend");

    // Reload and verify the selection persists
    await page.reload();
    const switcherAfterReload = page
      .locator("header")
      .getByRole("combobox")
      .first();
    await expect(switcherAfterReload).toContainText(/Westend/i);
  });

  test("storage type switcher shows Bulletin and Web3 Storage", async ({
    page,
  }) => {
    await page.goto("/");

    // Find the storage type selector (second combobox in header)
    const storageTypeSelector = page
      .locator("header")
      .getByRole("combobox")
      .nth(1);
    await storageTypeSelector.click();

    await expect(
      page.getByRole("option", { name: /Bulletin/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("option", { name: /Web3 Storage/i }),
    ).toBeVisible();
  });

  test("switching to Web3 Storage disables bulletin-only nav items", async ({
    page,
  }) => {
    await page.goto("/");

    // Switch to Web3 Storage
    const storageTypeSelector = page
      .locator("header")
      .getByRole("combobox")
      .nth(1);
    await storageTypeSelector.click();
    await page.getByRole("option", { name: /Web3 Storage/i }).click();

    // Faucet, Upload, Download, Renew should be disabled in web3storage mode
    const nav = page.locator("header nav");
    await expect(nav.getByRole("button", { name: "Faucet" })).toBeDisabled();
    await expect(nav.getByRole("button", { name: "Upload" })).toBeDisabled();
    await expect(
      nav.getByRole("button", { name: "Download" }),
    ).toBeDisabled();
    await expect(nav.getByRole("button", { name: "Renew" })).toBeDisabled();

    // Dashboard and Explorer should still work
    await expect(nav.getByText("Dashboard")).toBeVisible();
    await expect(nav.getByText("Explorer")).toBeVisible();
  });

  test("persists storage type in localStorage", async ({ page }) => {
    await page.goto("/");

    // Switch to Web3 Storage
    const storageTypeSelector = page
      .locator("header")
      .getByRole("combobox")
      .nth(1);
    await storageTypeSelector.click();
    await page.getByRole("option", { name: /Web3 Storage/i }).click();

    const storedType = await page.evaluate(() =>
      localStorage.getItem("bulletin-storage-type"),
    );
    expect(storedType).toBe("web3storage");
  });
});
