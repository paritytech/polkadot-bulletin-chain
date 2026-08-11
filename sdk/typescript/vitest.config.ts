// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

import { defineConfig } from "vitest/config"

export default defineConfig({
  test: {
    globals: true,
    environment: "node",
    coverage: {
      provider: "v8",
      reporter: ["text", "json", "html"],
      exclude: [
        "node_modules/**",
        "dist/**",
        "examples/**",
        "test/**",
        "*.config.ts",
      ],
    },
    // Unit test budgets; the integration suite sets its own larger
    // timeouts derived from the SDK's per-transaction timeout.
    testTimeout: 30000,
    hookTimeout: 30000,
  },
})
