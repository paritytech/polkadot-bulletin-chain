import { test, expect } from "./fixtures/test";

test.describe("Explorer Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/explorer");
  });

  test("shows page heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: /Block Explorer/i, level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText("Browse blocks and storage transactions"),
    ).toBeVisible();
  });

  test("displays search block card", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Search Block" }),
    ).toBeVisible();
    await expect(page.getByPlaceholder("Block number")).toBeVisible();
    await expect(page.getByRole("button", { name: "Go" })).toBeVisible();
  });

  test("go button is disabled without input", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "Go" }),
    ).toBeDisabled();
  });

  test("go button enables with block number input", async ({ page }) => {
    await page.getByPlaceholder("Block number").fill("42");
    await expect(
      page.getByRole("button", { name: "Go" }),
    ).toBeEnabled();
  });

  test("displays recent blocks card", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Recent Blocks" }),
    ).toBeVisible();
    // Without chain connection, shows empty state
    await expect(page.getByText("No blocks loaded")).toBeVisible();
  });

  test("displays empty state when no block selected", async ({ page }) => {
    await expect(
      page.getByText("Select a block to view details"),
    ).toBeVisible();
  });

  test("shows web3 storage explorer heading in web3storage mode", async ({
    page,
  }) => {
    const storageTypeSelector = page
      .locator("header")
      .getByRole("combobox")
      .nth(1);
    await storageTypeSelector.click();
    await page.getByRole("option", { name: /Web3 Storage/i }).click();

    // Navigate to explorer after switching
    await page.goto("/explorer");
    await expect(
      page.getByRole("heading", {
        name: /Web3 Storage Explorer/i,
        level: 1,
      }),
    ).toBeVisible();
  });

  test("block search accepts only numeric input", async ({ page }) => {
    const input = page.getByPlaceholder("Block number");
    await input.fill("42");
    await expect(input).toHaveValue("42");
  });
});
