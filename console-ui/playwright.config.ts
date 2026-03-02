import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [["html", { open: "never" }]],
  use: {
    baseURL: "http://localhost:5173",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "on-first-retry",
  },
  projects: [
    {
      name: "mock",
      use: { ...devices["Desktop Chrome"] },
      testIgnore: /integration/,
    },
    {
      name: "integration",
      use: { ...devices["Desktop Chrome"] },
      testMatch: /integration/,
      // Transaction tests share Alice's signing key — run serially to avoid nonce conflicts
      fullyParallel: false,
    },
  ],
  webServer: {
    command: "npm run dev",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
  },
});
