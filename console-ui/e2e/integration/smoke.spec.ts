/**
 * Smoke tests — read-only checks that run in parallel.
 *
 * These tests only read UI state and never submit chain transactions,
 * so they can safely run concurrently on separate workers.
 *
 * Prerequisites:
 *   A local parachain network via `just dev` or `just test-e2e`
 *   (node running on ws://localhost:10000 with IPFS enabled)
 */
import { test, expect } from "../fixtures";
import { waitForConnection } from "../utils";

test.describe.configure({ mode: "parallel" });

test.describe("Smoke Tests", () => {
  test("dashboard shows chain info and blocks increment", async ({
    localPage: page,
  }) => {
    // Chain info should be populated
    await expect(page.getByText("Chain Info")).toBeVisible();
    await expect(page.getByTestId("block-number")).toBeVisible();

    // Wait for block number to change (blocks produced every ~6s)
    const blockBadge = page.getByTestId("block-number");
    const initialText = await blockBadge.textContent();
    await expect(async () => {
      const currentText = await blockBadge.textContent();
      expect(currentText).not.toBe(initialText);
    }).toPass({ timeout: 15_000 });
  });

  test("upload page loads with chain connected", async ({
    localPage: page,
  }) => {
    // Upload nav link is disabled without wallet auth, navigate directly
    await page.goto("/upload");
    await expect(
      page.getByRole("heading", { name: "Upload Data" }),
    ).toBeVisible();

    // Block number in header confirms chain is still connected
    await waitForConnection(page);

    // Text input should be available
    await expect(
      page.getByPlaceholder("Enter data to store..."),
    ).toBeVisible();
  });

  test("download page loads with local network defaults", async ({
    localPage: page,
  }) => {
    await page.getByRole("link", { name: "Download", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Download Data" }),
    ).toBeVisible();

    // P2P tab should show local dev multiaddr by default
    await page.getByRole("tab", { name: /P2P Connection/i }).click();
    const textarea = page.getByTestId("peer-multiaddrs");
    await expect(textarea).toHaveValue(/127\.0\.0\.1/, { timeout: 10_000 });

    // Gateway tab should show local IPFS gateway URL
    await page.getByRole("tab", { name: /IPFS Gateway/i }).click();
    const gatewayInput = page.getByTestId("gateway-url-input");
    await expect(gatewayInput).toHaveValue("http://127.0.0.1:8283", {
      timeout: 10_000,
    });
  });

  test("upload button disabled without authorization", async ({
    page,
  }) => {
    // Use a bare page (no localPage fixture) — no chain connection set up,
    // so canUpload is always false regardless of on-chain state.
    await page.goto("/upload");
    await expect(
      page.getByRole("heading", { name: "Upload Data" }),
    ).toBeVisible();

    await page.getByPlaceholder("Enter data to store...").fill("test data");

    const uploadButton = page.getByTestId("upload-button");
    await expect(uploadButton).toBeDisabled();
  });
});
