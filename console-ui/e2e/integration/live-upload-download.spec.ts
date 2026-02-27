/**
 * Integration tests for upload and download on a live Bulletin Chain dev node.
 *
 * For full user journey tests (faucet → upload → download), see user-flows.spec.ts.
 *
 * Run with: npx playwright test --project=integration
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --ipfs-server --rpc-port 10000
 *   (node must be running on ws://localhost:10000 with IPFS enabled)
 */
import { test, expect, type Page } from "@playwright/test";

test.setTimeout(120_000);

/**
 * Set localStorage before any JS runs so the app initializes with "local"
 * network from the start (no race with the default Paseo auto-connect).
 */
async function setupLocalDev(page: Page) {
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

test.describe("Live Upload Page", () => {
  test.beforeEach(async ({ page }) => {
    await setupLocalDev(page);
  });

  test("upload page loads with chain connected", async ({ page }) => {
    // Upload nav link is disabled without wallet auth, navigate directly
    await page.goto("/upload");
    await expect(
      page.getByRole("heading", { name: "Upload Data" }),
    ).toBeVisible();

    // Block number in header confirms chain is still connected
    await expect(page.locator("header .font-mono")).toBeVisible({
      timeout: 10_000,
    });

    // Text input should be available
    await expect(
      page.getByPlaceholder("Enter data to store..."),
    ).toBeVisible();
  });

  test("upload button shows disabled without authorization", async ({
    page,
  }) => {
    // Upload nav link is disabled without wallet auth, navigate directly
    await page.goto("/upload");

    // Enter some text
    await page.getByPlaceholder("Enter data to store...").fill("test data");

    // Without authorization, upload should be disabled
    const uploadButton = page.getByRole("button", {
      name: /Upload to Bulletin Chain/,
    });
    await expect(uploadButton).toBeDisabled();
  });
});

test.describe("Live Download Page", () => {
  test.beforeEach(async ({ page }) => {
    await setupLocalDev(page);
  });

  test("download page loads with local network defaults", async ({
    page,
  }) => {
    await page.getByRole("link", { name: "Download", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Download Data" }),
    ).toBeVisible();

    // P2P tab should show local dev multiaddr by default
    await page.getByRole("tab", { name: /P2P Connection/i }).click();
    const textarea = page.locator("textarea");
    await expect(textarea).toHaveValue(/127\.0\.0\.1/, { timeout: 10_000 });
  });

  test("gateway tab shows local IPFS gateway URL", async ({ page }) => {
    await page.getByRole("link", { name: "Download", exact: true }).click();
    await page.getByRole("tab", { name: /IPFS Gateway/i }).click();

    const gatewayInput = page.locator(
      "input[placeholder='https://ipfs.example.com']",
    );
    await expect(gatewayInput).toHaveValue("http://127.0.0.1:8283", {
      timeout: 10_000,
    });
  });
});
