/**
 * Integration tests for full user flows on a live Bulletin Chain dev node.
 *
 * These tests exercise the UI the same way a real user would:
 * authorize storage, upload data, and download it back.
 *
 * Run with: npx playwright test --project=integration
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --ipfs-server --rpc-port 10000
 *   (node must be running on ws://localhost:10000 with IPFS enabled)
 */
import { test, expect, type Page } from "@playwright/test";

// Chain transactions take ~6s per block; generous timeout for multi-step flows
test.setTimeout(180_000);

/**
 * Set localStorage before any JS runs so the app initializes with "local"
 * network from the start, then navigate to "/" and wait for chain connection.
 */
async function connectToLocalDev(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem("bulletin-storage-type", "bulletin");
    localStorage.setItem("bulletin-network", "local");
  });
  await page.goto("/");
  // Block number badge only appears when connected + block received
  await expect(page.locator("header .font-mono")).toBeVisible({
    timeout: 30_000,
  });
}

/** Wait for chain connection after SPA navigation (block number in header). */
async function waitForConnection(page: Page) {
  await expect(page.locator("header .font-mono")).toBeVisible({
    timeout: 30_000,
  });
}

/**
 * Wait for the chain to produce enough blocks so that mortal transactions
 * have a valid era. On a freshly started --dev chain, submitting at block 1
 * can produce "Stale" errors because the mortality checkpoint is too recent.
 */
async function waitForMinBlock(page: Page, minBlock = 3) {
  await expect(async () => {
    const text = await page.locator("header .font-mono").textContent();
    const num = parseInt(text?.replace(/[#,]/g, "") ?? "0", 10);
    expect(num).toBeGreaterThanOrEqual(minBlock);
  }).toPass({ timeout: 30_000 });
}

/** Click a nav link in the header. Uses exact match to avoid Dashboard quick-action links. */
async function navigateTo(page: Page, name: string) {
  await page.locator("nav").getByRole("link", { name, exact: true }).click();
}

test.describe("Preimage Authorization Flow", () => {
  test("authorize preimage and upload data", async ({ page }) => {
    const testData = `Hello Bulletin Chain! Integration test ${Date.now()}`;

    await connectToLocalDev(page);

    // ── Step 1: Authorize preimage via Faucet ──────────────────────

    await test.step("authorize preimage via faucet", async () => {
      await navigateTo(page, "Faucet");
      await expect(
        page.getByRole("heading", { name: "Storage Faucet" }),
      ).toBeVisible({ timeout: 15_000 });

      // Wait for enough blocks so mortal transactions have a valid era
      await waitForMinBlock(page);

      // Switch to "Authorize Preimage" sub-tab
      await page.getByRole("tab", { name: /Authorize Preimage/i }).click();

      // Enter text – blake2 hash is auto-computed
      await page
        .getByPlaceholder("Enter text to compute blake2 hash...")
        .fill(testData);

      // Wait for the hash field to be populated
      const hashInput = page.locator(
        "input[placeholder='0x... (32 bytes hex)']",
      );
      await expect(hashInput).not.toHaveValue("", { timeout: 5_000 });

      // Submit authorization (Alice signs internally)
      await page
        .getByRole("button", { name: "Authorize Preimage" })
        .click();

      // Wait for on-chain confirmation
      await expect(
        page.getByText("Successfully authorized preimage"),
      ).toBeVisible({ timeout: 30_000 });
    });

    // ── Step 2: Upload with preimage auth (unsigned) ───────────────

    await test.step("upload data using preimage authorization", async () => {
      // Upload nav link is disabled without wallet auth; navigate directly
      await page.goto("/upload");
      await expect(
        page.getByRole("heading", { name: "Upload Data" }),
      ).toBeVisible();

      // Wait for chain connection after navigation
      await waitForConnection(page);

      // Enter the same text data
      await page.getByPlaceholder("Enter data to store...").fill(testData);

      // Wait for preimage authorization to be detected — the upload button
      // becomes enabled once the chain confirms the preimage auth.
      // Allow extra time: 300ms debounce + chain query + possible API re-sync.
      const uploadButton = page.getByRole("button", {
        name: /Upload to Bulletin Chain/i,
      });
      await expect(uploadButton).toBeEnabled({ timeout: 30_000 });

      // Upload
      await uploadButton.click();

      // Wait for on-chain confirmation
      await expect(page.getByText("Upload Successful")).toBeVisible({
        timeout: 30_000,
      });

      // Extract CID from the result card
      const cidInput = page.locator("input[readonly]").first();
      const uploadedCid = await cidInput.inputValue();
      expect(uploadedCid).toBeTruthy();
      expect(uploadedCid.length).toBeGreaterThan(10);
    });
  });
});

test.describe("Account Authorization Flow", () => {
  const ALICE_ADDRESS =
    "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";

  test("authorize account via faucet and verify in lookup", async ({
    page,
  }) => {
    await connectToLocalDev(page);

    await test.step("authorize account via faucet", async () => {
      await navigateTo(page, "Faucet");
      await expect(
        page.getByRole("heading", { name: "Storage Faucet" }),
      ).toBeVisible({ timeout: 15_000 });

      // Wait for enough blocks so mortal transactions have a valid era
      await waitForMinBlock(page);

      // "Authorize Account" sub-tab is active by default
      await expect(
        page.getByRole("heading", { name: "Authorize Account" }),
      ).toBeVisible();

      // Enter Alice's address
      await page
        .getByPlaceholder("Enter SS58 address...")
        .fill(ALICE_ADDRESS);

      // Wait for current authorization info to load
      await expect(page.getByText("Current Authorization:")).toBeVisible({
        timeout: 10_000,
      });

      // Set authorization amounts
      const txInput = page.locator(
        "input[type='number'][placeholder='Number of transactions']",
      );
      await txInput.fill("50");

      // Submit (Alice authorizes herself via sudo)
      await page
        .getByRole("button", { name: "Authorize Account" })
        .click();

      // Wait for on-chain confirmation
      await expect(
        page.getByText(/Successfully authorized account/),
      ).toBeVisible({ timeout: 30_000 });
    });

    await test.step("verify authorization in account lookup", async () => {
      // Switch to the "Accounts" main tab
      await page
        .getByRole("tab", { name: "Accounts", exact: true })
        .click();

      // Wait for the Lookup Account card to be visible
      await expect(page.getByText("Lookup Account")).toBeVisible({
        timeout: 5_000,
      });

      // The active tab panel contains the visible SS58 input;
      // use the Radix data-state attribute to scope to the active panel
      const activePanel = page.locator(
        '[role="tabpanel"][data-state="active"]',
      );
      await activePanel
        .getByPlaceholder("Enter SS58 address...")
        .fill(ALICE_ADDRESS);

      const searchButton = activePanel.getByRole("button", { name: "Search" });
      await expect(searchButton).toBeEnabled({ timeout: 5_000 });
      await searchButton.click();

      // Verify authorization details appear
      await expect(page.getByText("Transactions").first()).toBeVisible({
        timeout: 10_000,
      });
      await expect(page.getByText("Bytes").first()).toBeVisible();
    });
  });
});

test.describe("Preimage Authorization Listing", () => {
  test("authorized preimage appears in preimage list", async ({ page }) => {
    const testData = `Preimage listing test ${Date.now()}`;

    await connectToLocalDev(page);

    // Navigate to faucet
    await navigateTo(page, "Faucet");
    await expect(
      page.getByRole("heading", { name: "Storage Faucet" }),
    ).toBeVisible({ timeout: 15_000 });

    // Wait for enough blocks so mortal transactions have a valid era
    await waitForMinBlock(page);

    await page.getByRole("tab", { name: /Authorize Preimage/i }).click();
    await page
      .getByPlaceholder("Enter text to compute blake2 hash...")
      .fill(testData);

    // Wait for hash to compute
    const hashInput = page.locator(
      "input[placeholder='0x... (32 bytes hex)']",
    );
    await expect(hashInput).not.toHaveValue("", { timeout: 5_000 });
    const expectedHash = await hashInput.inputValue();

    await page.getByRole("button", { name: "Authorize Preimage" }).click();
    await expect(
      page.getByText("Successfully authorized preimage"),
    ).toBeVisible({ timeout: 30_000 });

    // Switch to Preimages tab and verify the hash is listed
    await page
      .getByRole("tab", { name: "Preimages", exact: true })
      .click();

    // Scope to the active tab panel to avoid matching the Renew nav button's
    // RefreshCw icon in the header (both use svg.lucide-refresh-cw).
    const activePanel = page.locator(
      '[role="tabpanel"][data-state="active"]',
    );

    // Wait for the preimage list to load and show our hash.
    // Use retry with refresh button in case the list is stale from a prior test.
    await expect(async () => {
      // Click refresh to re-fetch from chain
      const refreshButton = activePanel.locator(
        "button:has(svg.lucide-refresh-cw)",
      );
      if (await refreshButton.isVisible()) {
        await refreshButton.click();
      }
      await expect(
        activePanel.getByText(expectedHash.slice(0, 16)),
      ).toBeVisible({ timeout: 5_000 });
    }).toPass({ timeout: 30_000, intervals: [2_000, 5_000, 5_000] });
  });
});

test.describe("Dashboard Live Data", () => {
  test("dashboard shows chain info after connection", async ({ page }) => {
    await connectToLocalDev(page);

    // Dashboard is the landing page
    await expect(page.getByText("Chain Info")).toBeVisible({
      timeout: 15_000,
    });

    // Spec version and other chain info should be populated
    await expect(page.getByText(/spec.*version/i).first()).toBeVisible({
      timeout: 15_000,
    });
  });
});
