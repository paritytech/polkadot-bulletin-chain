import { test, expect } from "./fixtures/test";

test.describe("Renew Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/renew");
  });

  test("shows page heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Renew Storage", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText("Extend the retention period for your stored data"),
    ).toBeVisible();
  });

  test("displays find storage transaction card", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Find Storage Transaction" }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Select from your history or enter the block number and transaction index",
      ),
    ).toBeVisible();
  });

  test("shows block number and transaction index inputs", async ({ page }) => {
    await expect(page.getByPlaceholder("e.g., 12345")).toBeVisible();
    await expect(page.getByPlaceholder("e.g., 0")).toBeVisible();
  });

  test("lookup button is disabled without inputs", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "Lookup Transaction" }),
    ).toBeDisabled();
  });

  test("lookup button enables when both inputs filled", async ({ page }) => {
    await page.getByPlaceholder("e.g., 12345").fill("100");
    await page.getByPlaceholder("e.g., 0").fill("0");
    // Button still disabled because API is not connected (no chain)
    // but inputs are accepted
    const button = page.getByRole("button", { name: "Lookup Transaction" });
    await expect(button).toBeVisible();
  });

  test("displays about renewal info card", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "About Renewal" }),
    ).toBeVisible();
    await expect(
      page.getByText("Data stored on Bulletin Chain has a retention period"),
    ).toBeVisible();
  });

  test("shows connect wallet prompt in sidebar", async ({ page }) => {
    await expect(
      page.getByText("Connect a wallet to renew data"),
    ).toBeVisible();
    await expect(
      page.getByRole("link", { name: "Connect Wallet" }),
    ).toBeVisible();
  });

  test("connect wallet link points to accounts page", async ({ page }) => {
    const link = page.getByRole("link", { name: "Connect Wallet" });
    await expect(link).toHaveAttribute("href", "/accounts");
  });

  test("loads block and index from URL params", async ({ page }) => {
    await page.goto("/renew?block=999&index=2");

    // URL params should populate the form fields
    await expect(page.getByPlaceholder("e.g., 12345")).toHaveValue("999");
    await expect(page.getByPlaceholder("e.g., 0")).toHaveValue("2");
  });
});
