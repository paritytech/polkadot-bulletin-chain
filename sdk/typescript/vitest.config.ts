// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

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
    testTimeout: 60000, // 60 seconds for integration tests
    hookTimeout: 60000,
  },
})
