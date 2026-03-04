/**
 * Integration tests for upload and download on a live Bulletin Chain dev node.
 *
 * For full user journey tests (faucet -> upload -> download), see user-flows.spec.ts.
 *
 * Run with: playwright test
 *
 * Prerequisites:
 *   ./target/release/polkadot-bulletin-chain --dev --ipfs-server --rpc-port 10000
 *   (node must be running on ws://localhost:10000 with IPFS enabled)
 */
import { test, expect } from "../fixtures";
import { waitForConnection } from "../utils";

test.setTimeout(120_000);

test.describe("Live Upload Page", () => {
  test("upload page loads with chain connected", async ({ localPage: page }) => {
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

  test("upload button shows disabled without authorization", async ({
    localPage: page,
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
  });

  test("gateway tab shows local IPFS gateway URL", async ({ localPage: page }) => {
    await page.getByRole("link", { name: "Download", exact: true }).click();
    await page.getByRole("tab", { name: /IPFS Gateway/i }).click();

    const gatewayInput = page.getByTestId("gateway-url-input");
    await expect(gatewayInput).toHaveValue("http://127.0.0.1:8283", {
      timeout: 10_000,
    });
  });
});
