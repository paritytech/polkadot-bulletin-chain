import { test, expect } from "./fixtures/test";

test.describe("Navigation", () => {
  test("dashboard route renders", async ({ page }) => {
    await page.goto("/");
    await expect(
      page.getByRole("heading", { name: "Dashboard", level: 1 }),
    ).toBeVisible();
  });

  test("upload route renders", async ({ page }) => {
    await page.goto("/upload");
    await expect(
      page.getByRole("heading", { name: "Upload Data", level: 1 }),
    ).toBeVisible();
  });

  test("download route renders", async ({ page }) => {
    await page.goto("/download");
    await expect(
      page.getByRole("heading", { name: "Download Data", level: 1 }),
    ).toBeVisible();
  });

  test("renew route renders", async ({ page }) => {
    await page.goto("/renew");
    await expect(
      page.getByRole("heading", { name: /Renew/i, level: 1 }),
    ).toBeVisible();
  });

  test("explorer route renders", async ({ page }) => {
    await page.goto("/explorer");
    await expect(
      page.getByRole("heading", { name: /Explorer/i, level: 1 }),
    ).toBeVisible();
  });

  test("authorizations route renders", async ({ page }) => {
    await page.goto("/authorizations");
    await expect(
      page.getByRole("heading", { name: /Authorizations|Faucet/i, level: 1 }),
    ).toBeVisible();
  });

  test("accounts route renders", async ({ page }) => {
    await page.goto("/accounts");
    await expect(
      page.getByRole("heading", { name: /Accounts|Wallet/i, level: 1 }),
    ).toBeVisible();
  });

  test("unknown route redirects to dashboard", async ({ page }) => {
    await page.goto("/nonexistent-route");
    await expect(
      page.getByRole("heading", { name: "Dashboard", level: 1 }),
    ).toBeVisible();
    expect(page.url()).not.toContain("nonexistent-route");
  });

  test("can navigate between pages via header nav", async ({ page }) => {
    await page.goto("/");
    await expect(
      page.getByRole("heading", { name: "Dashboard", level: 1 }),
    ).toBeVisible();

    // Navigate to Explorer (always enabled)
    await page.getByRole("link", { name: "Explorer" }).click();
    await expect(
      page.getByRole("heading", { name: /Explorer/i, level: 1 }),
    ).toBeVisible();

    // Navigate to Download (always enabled)
    await page.getByRole("link", { name: "Download" }).click();
    await expect(
      page.getByRole("heading", { name: "Download Data", level: 1 }),
    ).toBeVisible();

    // Navigate back to Dashboard via logo
    await page.getByRole("link", { name: /Bulletin Chain/i }).click();
    await expect(
      page.getByRole("heading", { name: "Dashboard", level: 1 }),
    ).toBeVisible();
  });
});
