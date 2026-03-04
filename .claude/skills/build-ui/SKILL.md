---
name: build-ui
description: Build the console UI (same as CI)
---

Build the Bulletin Chain web dashboard (`console-ui/`) exactly as CI does. From the `console-ui/` directory:

1. `npm ci`
2. `npx papi generate` — skip this step if `$ARGUMENTS` contains `--skip-papi`
3. `GITHUB_PAGES=true npm run build`

Report any TypeScript or build errors found.
