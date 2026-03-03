import { test, expect } from "./fixtures/test";

test.describe("Authorizations Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/authorizations");
  });

  test("shows page heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Faucet & Authorizations", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText("Storage faucet and authorization management"),
    ).toBeVisible();
  });

  test("displays three main tabs", async ({ page }) => {
    await expect(
      page.getByRole("tab", { name: /Storage Faucet/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("tab", { name: "Accounts", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("tab", { name: "Preimages", exact: true }),
    ).toBeVisible();
  });

  test("faucet tab is active by default", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Storage Faucet" }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Authorize storage allowances using the Alice dev account",
      ),
    ).toBeVisible();
  });

  test("faucet has account and preimage sub-tabs", async ({ page }) => {
    await expect(
      page.getByRole("tab", { name: /Authorize Account/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /Authorize Preimage/ }),
    ).toBeVisible();
  });

  test("authorize account sub-tab shows form fields", async ({ page }) => {
    // Account sub-tab is active by default within faucet
    await expect(
      page.getByRole("heading", { name: "Authorize Account" }),
    ).toBeVisible();
    await expect(page.getByPlaceholder("Enter SS58 address...")).toBeVisible();
    await expect(
      page.getByPlaceholder("Number of transactions"),
    ).toBeVisible();
    await expect(page.getByPlaceholder("Amount")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Authorize Account" }),
    ).toBeVisible();
  });

  test("authorize preimage sub-tab shows form fields", async ({ page }) => {
    await page.getByRole("tab", { name: /Authorize Preimage/ }).click();

    await expect(
      page.getByRole("heading", { name: "Authorize Preimage" }),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder("0x... (32 bytes hex)"),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder("Enter text to compute blake2 hash..."),
    ).toBeVisible();
    await expect(page.getByPlaceholder("Maximum size")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Authorize Preimage" }),
    ).toBeVisible();
  });

  test("preimage sub-tab has text and file input modes", async ({ page }) => {
    await page.getByRole("tab", { name: /Authorize Preimage/ }).click();

    // Text tab is default
    await expect(
      page.getByPlaceholder("Enter text to compute blake2 hash..."),
    ).toBeVisible();

    // Switch to file tab
    await page.getByRole("tab", { name: "File", exact: true }).click();
    await expect(page.getByText(/drag and drop|click to browse/i)).toBeVisible();
  });

  test("preimage hash auto-computes from text input", async ({ page }) => {
    await page.getByRole("tab", { name: /Authorize Preimage/ }).click();

    const hashInput = page.getByPlaceholder("0x... (32 bytes hex)");
    await expect(hashInput).toHaveValue("");

    // Type text — hash should be computed
    await page
      .getByPlaceholder("Enter text to compute blake2 hash...")
      .fill("test data for hash");

    await expect(hashInput).not.toHaveValue("", { timeout: 5_000 });
    const hash = await hashInput.inputValue();
    expect(hash).toMatch(/^0x[0-9a-f]{64}$/);
  });

  test("preimage hash validation shows error for invalid input", async ({
    page,
  }) => {
    await page.getByRole("tab", { name: /Authorize Preimage/ }).click();

    // Type invalid hash
    await page
      .getByPlaceholder("0x... (32 bytes hex)")
      .fill("not-a-valid-hash");

    await expect(
      page.getByText("Must be a 32-byte hex string"),
    ).toBeVisible();
  });

  test("accounts tab shows authorization and lookup cards", async ({
    page,
  }) => {
    await page.getByRole("tab", { name: "Accounts", exact: true }).click();

    // Scope to active tab panel to avoid matching hidden faucet inputs
    const panel = page.locator('[role="tabpanel"][data-state="active"]');

    await expect(
      panel.getByRole("heading", { name: "Your Authorization" }),
    ).toBeVisible();
    await expect(
      panel.getByText("Connect a wallet to view your authorization"),
    ).toBeVisible();

    await expect(
      panel.getByRole("heading", { name: "Lookup Account" }),
    ).toBeVisible();
    await expect(
      panel.getByText("Check authorization for any account"),
    ).toBeVisible();
    await expect(
      panel.getByPlaceholder("Enter SS58 address..."),
    ).toBeVisible();
  });

  test("preimages tab shows authorization list", async ({ page }) => {
    await page.getByRole("tab", { name: "Preimages", exact: true }).click();

    await expect(
      page.getByRole("heading", { name: "Preimage Authorizations" }),
    ).toBeVisible();
    await expect(
      page.getByText("Content hashes authorized for unsigned uploads"),
    ).toBeVisible();
  });

  test("tab selection updates URL query parameter", async ({ page }) => {
    // Default tab uses fallback — URL has no tab param initially
    await page.getByRole("tab", { name: "Accounts", exact: true }).click();
    expect(page.url()).toContain("tab=accounts");

    await page.getByRole("tab", { name: "Preimages", exact: true }).click();
    expect(page.url()).toContain("tab=preimages");

    await page.getByRole("tab", { name: /Storage Faucet/ }).click();
    expect(page.url()).toContain("tab=faucet");
  });

  test("navigating with tab query param selects correct tab", async ({
    page,
  }) => {
    await page.goto("/authorizations?tab=preimages");
    await expect(
      page.getByRole("heading", { name: "Preimage Authorizations" }),
    ).toBeVisible();
  });

  test("max size unit selector shows options", async ({ page }) => {
    await page.getByRole("tab", { name: /Authorize Preimage/ }).click();

    const unitSelect = page.locator("select");
    await expect(unitSelect).toBeVisible();
    await expect(unitSelect.locator("option[value='B']")).toHaveText("Bytes");
    await expect(unitSelect.locator("option[value='KB']")).toHaveText("KB");
    await expect(unitSelect.locator("option[value='MB']")).toHaveText("MB");
  });
});
