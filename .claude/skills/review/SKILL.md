---
name: review
description: Review local changes or a pull request (authoritative review criteria)
---

This skill is the single source of truth for code review criteria in this repository.

If no arguments are passed, review the local changes by looking at the diff between the base branch - main by default - and the current branch.
If arguments are passed, review pull request #$ARGUMENTS by fetching it and seeing its details with `gh pr view` and `gh pr diff`.

When reviewing, analyze for:

1. **Code Quality**
   - Rust idioms and Polkadot SDK patterns
   - Error handling: use `Result` types with meaningful error enums; avoid `unwrap()` and `expect()` in production code (acceptable in tests)
   - Arithmetic safety: use `checked_*`, `saturating_*`, or `wrapping_*` arithmetic to prevent overflow; never use raw arithmetic operators on user-provided values
   - Naming: follow Rust conventions (snake_case for functions/variables, CamelCase for types)
   - Complexity: prefer simple, readable code; avoid over-engineering and premature abstractions
   - Comments should explain **why**, not **how**

2. **FRAME Pallet Standards**
   - Use appropriate storage types (`StorageValue`, `StorageMap`, `StorageDoubleMap`, `CountedStorageMap`)
   - Emit events for all state changes that external observers need to track
   - Define descriptive error types in the pallet's `Error` enum
   - All extrinsics must have accurate weight annotations; update benchmarks when logic changes
   - Use the principle of least privilege for origin checks
   - Be cautious with `on_initialize` and `on_finalize`: they affect block production time in solochains and can brick parachains; never panic or do unbounded iteration in them; always benchmark them properly

3. **Security**
   - Runtime code must never panic; use defensive programming
   - Use `BoundedVec`, `BoundedBTreeMap` etc. to prevent unbounded storage growth
   - Validate all user inputs at the entry point
   - Consider requiring deposits for user-created storage items
   - Review unsafe code blocks for soundness
   - Verify access control in pallets uses appropriate origin checks

4. **Performance**
   - Weight/benchmark implications
   - Storage access patterns
   - Unnecessary allocations

5. **Testing**
   - All new functionality requires unit tests
   - Test boundary conditions, error paths, and malicious inputs
   - Complex features need integration tests using `sp-io::TestExternalities`
   - Features affecting weights need benchmark tests

6. **PR Standards**
   - Single responsibility: each PR addresses one concern
   - All CI checks pass (`cargo test`, `cargo clippy`, `cargo fmt`)
   - Code compiles without warnings
   - Public APIs have rustdoc comments

7. **Breaking Changes**
   - API compatibility
   - Migration requirements

Provide specific feedback with file paths and line numbers.
