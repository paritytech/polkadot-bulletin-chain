import { test, expect } from "./fixtures/test";

test.describe("Header", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("displays logo with Bulletin Chain branding", async ({ page }) => {
    // The "B" logo icon
    await expect(page.locator("header").getByText("B").first()).toBeVisible();
    // The "Bulletin Chain" text (hidden on small screens, visible on sm+)
    await expect(
      page.locator("header").getByText("Bulletin Chain"),
    ).toBeVisible();
  });

  test("shows navigation menu items", async ({ page }) => {
    const nav = page.locator("header nav");

    // Dashboard and Explorer are always visible and enabled
    await expect(nav.getByText("Dashboard")).toBeVisible();
    await expect(nav.getByText("Explorer")).toBeVisible();

    // Faucet is visible (may be disabled in web3storage mode)
    await expect(nav.getByText("Faucet")).toBeVisible();

    // Download is visible
    await expect(nav.getByText("Download")).toBeVisible();
  });

  test("shows Connect wallet button when no wallet connected", async ({
    page,
  }) => {
    await expect(
      page.locator("header").getByRole("link", { name: /Connect/i }),
    ).toBeVisible();
  });

  test("shows network switcher", async ({ page }) => {
    // The network switcher is a Select component showing the current network
    await expect(
      page.locator("header").getByRole("combobox").first(),
    ).toBeVisible();
  });

  test("shows connection status indicator", async ({ page }) => {
    // The status indicator is a colored dot (div with rounded-full class)
    await expect(
      page.locator("header .rounded-full").first(),
    ).toBeVisible();
  });

  test("Upload and Renew are disabled without wallet", async ({ page }) => {
    const nav = page.locator("header nav");

    // Upload should be disabled (requires auth)
    const uploadButton = nav.getByRole("button", { name: "Upload" });
    await expect(uploadButton).toBeDisabled();

    // Renew should be disabled (requires auth)
    const renewButton = nav.getByRole("button", { name: "Renew" });
    await expect(renewButton).toBeDisabled();
  });
});
