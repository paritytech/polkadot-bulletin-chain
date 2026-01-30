---
name: review
description: Review local changes or a pull request
---

If no arguments are passed, review the local changes by looking at the diff between the base branch - main by default - and the current branch.
If arguments are passed, review pull request #$ARGUMENTS by fetching it and seeing its details with `gh pr view` and `gh pr diff`.

When reviewing, analyze for:

1. **Code Quality**
   - Rust idioms and Polkadot SDK patterns
   - Error handling and unwrap usage
   - Code clarity and maintainability

2. **Security**
   - Unsafe code blocks
   - Input validation
   - Access control in pallets

3. **Performance**
   - Weight/benchmark implications
   - Storage access patterns
   - Unnecessary allocations

4. **Testing**
   - Test coverage for new code
   - Edge cases handled

5. **Breaking Changes**
   - API compatibility
   - Migration requirements

Provide specific feedback with file paths and line numbers.
