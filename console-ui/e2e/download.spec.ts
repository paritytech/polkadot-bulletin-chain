import { test, expect } from "./fixtures/test";

test.describe("Download Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/download");
  });

  test("shows download heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Download Data", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText(
        /Retrieve data from the Bulletin Chain via P2P or IPFS Gateway/,
      ),
    ).toBeVisible();
  });

  test("displays P2P and Gateway tabs", async ({ page }) => {
    await expect(
      page.getByRole("tab", { name: /P2P Connection/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /IPFS Gateway/i }),
    ).toBeVisible();
  });

  test("P2P tab shows connection controls", async ({ page }) => {
    // The P2P tab might not be active by default (depends on network's preferred method)
    await page.getByRole("tab", { name: /P2P Connection/i }).click();

    await expect(page.getByText("Peer Multiaddrs")).toBeVisible();
    // Use the P2P panel's specific Connect button (not the header one)
    await expect(
      page.getByLabel("P2P Connection").getByRole("button", { name: /Connect/i }),
    ).toBeVisible();
  });

  test("Gateway tab shows gateway URL input", async ({ page }) => {
    await page.getByRole("tab", { name: /IPFS Gateway/i }).click();

    await expect(page.getByLabel("IPFS Gateway").getByText("Gateway URL", { exact: true })).toBeVisible();
  });

  test("shows Fetch by CID section", async ({ page }) => {
    await expect(page.getByText("Fetch by CID")).toBeVisible();
    await expect(page.getByText("CID", { exact: true })).toBeVisible();
  });

  test("shows CID Info card", async ({ page }) => {
    await expect(page.getByText("CID Info")).toBeVisible();
    await expect(
      page.getByText("Enter a valid CID to see details"),
    ).toBeVisible();
  });

  test("shows storage history card", async ({ page }) => {
    await expect(page.getByText("My Storage")).toBeVisible();
    await expect(
      page.getByText(/No storage history yet/),
    ).toBeVisible();
  });

  test("fetch button is disabled without data source", async ({ page }) => {
    const fetchButton = page.getByRole("button", { name: /Fetch Data/i });
    await expect(fetchButton).toBeDisabled();
  });

  test("loads CID from URL query parameter", async ({ page }) => {
    // Navigate with a CID in the URL
    const testCid =
      "bafkreihdwdcefgh4dqkjv67uzcmw7oje5d2hax73oldqhwmrnncsxa2yhi";
    await page.goto(`/download?cid=${testCid}`);

    // The CID input should be pre-filled
    const cidInput = page.locator("input").first();
    await expect(cidInput).toHaveValue(testCid);
  });
});
