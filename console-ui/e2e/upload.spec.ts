import { test, expect } from "./fixtures/test";

test.describe("Upload Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/upload");
  });

  test("shows upload heading and description", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Upload Data", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByText(
        /Store data on the Bulletin Chain and receive an IPFS-compatible CID/,
      ),
    ).toBeVisible();
  });

  test("displays Data Input card with Text and File tabs", async ({
    page,
  }) => {
    await expect(page.getByText("Data Input")).toBeVisible();
    await expect(page.getByRole("tab", { name: "Text" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "File" })).toBeVisible();
  });

  test("text tab shows textarea", async ({ page }) => {
    // Text tab should be active by default
    await expect(page.getByRole("tab", { name: "Text" })).toHaveAttribute(
      "data-state",
      "active",
    );
    await expect(
      page.getByPlaceholder("Enter data to store..."),
    ).toBeVisible();
  });

  test("can type text into the textarea", async ({ page }) => {
    const textarea = page.getByPlaceholder("Enter data to store...");
    await textarea.fill("Hello, Bulletin Chain!");
    await expect(textarea).toHaveValue("Hello, Bulletin Chain!");

    // Data size badge should appear
    await expect(page.getByText(/Data size:/)).toBeVisible();
  });

  test("file tab shows file upload area", async ({ page }) => {
    await page.getByRole("tab", { name: "File" }).click();
    await expect(page.getByRole("tab", { name: "File" })).toHaveAttribute(
      "data-state",
      "active",
    );
    // File upload area should be visible
    await expect(
      page.getByText(/drag|drop|choose|select|browse/i),
    ).toBeVisible();
  });

  test("displays CID Configuration card", async ({ page }) => {
    await expect(page.getByText("CID Configuration")).toBeVisible();
    await expect(page.getByText("Hash Algorithm")).toBeVisible();
    await expect(page.getByText("CID Codec")).toBeVisible();
  });

  test("upload button is disabled without authorization", async ({ page }) => {
    // No wallet connected and no authorization, so upload should be disabled
    const uploadButton = page.getByRole("button", {
      name: /Upload to Bulletin Chain/,
    });
    await expect(uploadButton).toBeDisabled();
  });

  test("shows connect wallet prompt when no wallet connected", async ({
    page,
  }) => {
    // The sidebar shows a prompt to connect a wallet or use pre-authorized data
    await expect(
      page.getByRole("link", { name: /Connect Wallet/i }).last(),
    ).toBeVisible();
  });
});
