/**
 * User flow tests — serial write tests that submit chain transactions.
 *
 * These tests share Alice's signing key and modify on-chain state,
 * so they must run serially to avoid nonce conflicts.
 *
 * Prerequisites:
 *   A local parachain network via `just dev` or `just test-e2e`
 *   (node running on ws://localhost:10000 with IPFS enabled)
 */
import { test, expect } from "../fixtures";
import { waitForConnection, waitForMinBlock, navigateTo } from "../utils";

// Chain transactions take ~6s per block; generous timeout for multi-step flows
test.setTimeout(180_000);

test.describe("Preimage Round-Trip", () => {
  test("authorize preimage, upload data, and download to verify content", async ({
    localPage: page,
    request,
  }) => {
    const testData = `Hello Bulletin Chain! Integration test ${Date.now()}`;
    let uploadedCid: string;

    // ── Step 1: Authorize preimage via Faucet ──────────────────────

    await test.step("authorize preimage via faucet", async () => {
      await navigateTo(page, "Faucet");
      await expect(
        page.getByRole("heading", { name: "Storage Faucet" }),
      ).toBeVisible({ timeout: 15_000 });

      await waitForMinBlock(page);

      await page.getByRole("tab", { name: /Authorize Preimage/i }).click();

      await page
        .getByPlaceholder("Enter text to compute blake2 hash...")
        .fill(testData);

      const hashInput = page.getByTestId("preimage-hash-input");
      await expect(hashInput).not.toHaveValue("", { timeout: 5_000 });

      await page
        .getByRole("button", { name: "Authorize Preimage" })
        .click();

      await expect(
        page.getByText("Successfully authorized preimage"),
      ).toBeVisible({ timeout: 60_000 });
    });

    // ── Step 2: Upload with preimage auth (unsigned) ───────────────

    await test.step("upload data using preimage authorization", async () => {
      await page.goto("/upload");
      await expect(
        page.getByRole("heading", { name: "Upload Data" }),
      ).toBeVisible();

      await waitForConnection(page);

      await page.getByPlaceholder("Enter data to store...").fill(testData);

      const uploadButton = page.getByRole("button", {
        name: /Upload to Bulletin Chain/i,
      });
      await expect(uploadButton).toBeEnabled({ timeout: 60_000 });

      await uploadButton.click();

      await expect(page.getByText("Upload Successful")).toBeVisible({
        timeout: 60_000,
      });

      const cidDisplay = page.getByTestId("cid-display");
      uploadedCid = await cidDisplay.inputValue();
      expect(uploadedCid).toBeTruthy();
      expect(uploadedCid.length).toBeGreaterThan(10);
    });

    // ── Step 3: Download via IPFS gateway and verify content ───────

    await test.step("download and verify content matches", async () => {
      // Fetch directly from the local IPFS gateway
      const response = await request.get(
        `http://127.0.0.1:8283/ipfs/${uploadedCid}`,
        { timeout: 60_000 },
      );
      expect(response.ok()).toBeTruthy();

      const downloadedText = await response.text();
      expect(downloadedText).toBe(testData);
    });
  });
});

test.describe("Account Authorization Flow", () => {
  const ALICE_ADDRESS =
    "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";

  test("authorize account via faucet and verify in lookup", async ({
    localPage: page,
  }) => {
    // Note: upload with account auth requires a connected wallet extension,
    // which is not available in headless CI. This test verifies the faucet
    // authorization and account lookup only.

    await test.step("authorize account via faucet", async () => {
      await navigateTo(page, "Faucet");
      await expect(
        page.getByRole("heading", { name: "Storage Faucet" }),
      ).toBeVisible({ timeout: 15_000 });

      await waitForMinBlock(page);

      await expect(
        page.getByRole("heading", { name: "Authorize Account" }),
      ).toBeVisible();

      await page
        .getByPlaceholder("Enter SS58 address...")
        .fill(ALICE_ADDRESS);

      await expect(page.getByText("Current Authorization:")).toBeVisible({
        timeout: 10_000,
      });

      const txInput = page.locator(
        "input[type='number'][placeholder='Number of transactions']",
      );
      await txInput.fill("50");

      await page
        .getByRole("button", { name: "Authorize Account" })
        .click();

      await expect(
        page.getByText(/Successfully authorized account/),
      ).toBeVisible({ timeout: 60_000 });
    });

    await test.step("verify authorization in account lookup", async () => {
      await page
        .getByRole("tab", { name: "Accounts", exact: true })
        .click();

      await expect(page.getByText("Lookup Account")).toBeVisible({
        timeout: 5_000,
      });

      const activePanel = page.locator(
        '[role="tabpanel"][data-state="active"]',
      );
      await activePanel
        .getByPlaceholder("Enter SS58 address...")
        .fill(ALICE_ADDRESS);

      const searchButton = activePanel.getByRole("button", { name: "Search" });
      await expect(searchButton).toBeEnabled({ timeout: 5_000 });
      await searchButton.click();

      await expect(page.getByText("Transactions").first()).toBeVisible({
        timeout: 10_000,
      });
      await expect(page.getByText("Bytes").first()).toBeVisible();
    });
  });
});

test.describe("Preimage Listing", () => {
  test("authorized preimage appears in preimage list", async ({
    localPage: page,
  }) => {
    const testData = `Preimage listing test ${Date.now()}`;

    await navigateTo(page, "Faucet");
    await expect(
      page.getByRole("heading", { name: "Storage Faucet" }),
    ).toBeVisible({ timeout: 15_000 });

    await waitForMinBlock(page);

    await page.getByRole("tab", { name: /Authorize Preimage/i }).click();
    await page
      .getByPlaceholder("Enter text to compute blake2 hash...")
      .fill(testData);

    const hashInput = page.getByTestId("preimage-hash-input");
    await expect(hashInput).not.toHaveValue("", { timeout: 5_000 });
    const expectedHash = await hashInput.inputValue();

    await page.getByRole("button", { name: "Authorize Preimage" }).click();
    await expect(
      page.getByText("Successfully authorized preimage"),
    ).toBeVisible({ timeout: 60_000 });

    // Switch to Preimages tab and verify the hash is listed
    await page
      .getByRole("tab", { name: "Preimages", exact: true })
      .click();

    const activePanel = page.locator(
      '[role="tabpanel"][data-state="active"]',
    );

    await expect(async () => {
      const refreshButton = activePanel.getByTestId("refresh-preimage-list");
      if (await refreshButton.isVisible()) {
        await refreshButton.click();
      }
      await expect(
        activePanel.getByText(expectedHash.slice(0, 16)),
      ).toBeVisible({ timeout: 5_000 });
    }).toPass({ timeout: 30_000, intervals: [2_000, 5_000, 5_000] });
  });
});
