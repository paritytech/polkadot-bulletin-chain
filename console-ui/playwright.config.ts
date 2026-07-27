// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { defineConfig, devices } from "@playwright/test";
import type { TestOptions } from "./e2e/fixtures";

// Collator-1 of zombienet/bulletin-westend-local.toml (deterministic node
// key). `just test` overrides this per runtime via COLLATOR_MULTIADDR.
const DEFAULT_COLLATOR_MULTIADDR =
  "/ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ";

export default defineConfig<TestOptions>({
  testDir: "./e2e",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 2,
  reporter: [["html", { open: "never" }]],
  use: {
    baseURL: "http://localhost:5173",
    collatorMultiaddr: process.env.COLLATOR_MULTIADDR ?? DEFAULT_COLLATOR_MULTIADDR,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "on-first-retry",
    ...devices["Desktop Chrome"],
  },
  webServer: {
    command: "npm run dev",
    url: "http://localhost:5173",
    reuseExistingServer: !process.env.CI,
  },
});
