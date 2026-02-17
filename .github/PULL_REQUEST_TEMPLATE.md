## Description

Please include a summary of the changes and the related issue. Include relevant motivation and context.

Fixes # (issue)

## Type of Change

Please delete options that are not relevant.

- [ ] 🐛 Bug fix (non-breaking change which fixes an issue)
- [ ] ✨ New feature (non-breaking change which adds functionality)
- [ ] 💥 Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] 📚 Documentation update
- [ ] 🔧 Refactoring (no functional changes)
- [ ] ⚡ Performance improvement

## How Has This Been Tested?

Please describe the tests that you ran to verify your changes. Provide instructions so we can reproduce.

- [ ] Unit tests
- [ ] Integration tests
- [ ] Manual testing

**Test Configuration:**
- OS:
- Rust version:

## Checklist

- [ ] My code follows the style guidelines of this project
- [ ] I have performed a self-review of my code
- [ ] I have commented my code, particularly in hard-to-understand areas
- [ ] I have made corresponding changes to the documentation
- [ ] My changes generate no new warnings
- [ ] I have added tests that prove my fix is effective or that my feature works
- [ ] New and existing unit tests pass locally with my changes
- [ ] Any dependent changes have been merged and published

## Unsafe Code Checklist

> **Skip this section if your PR does not add or modify `unsafe` code.**
> See [docs/UNSAFE_REVIEW_CHECKLIST.md](../docs/UNSAFE_REVIEW_CHECKLIST.md) for details.

- [ ] All `unsafe fn` have `# Safety` documentation describing preconditions
- [ ] All `unsafe {}` blocks have `// SAFETY:` comments explaining why it's sound
- [ ] Invariants documented in [docs/SOUNDNESS.md](../docs/SOUNDNESS.md) (if new unsafe module)
- [ ] No undefined behavior possible with valid inputs
- [ ] Edge cases tested (empty, boundary values, alignment)
- [ ] Miri tests pass (if applicable): `cargo +nightly miri test <test_name>`

## Screenshots (if applicable)

Add screenshots to help explain your changes.

## Additional Notes

Add any other notes about the PR here.

## Performance Labels

- ARM64 benchmark is cost-gated on PRs.
- Add label `run-arm64-bench` to run the full ARM64 benchmark workflow for this PR.
