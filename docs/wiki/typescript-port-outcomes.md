# Identity comparisons during TypeScript ports

On 2026-11-13, review of [PR #387](https://github.com/Duresa7/OpenTouchDeck/pull/387) found that a TypeScript notice-object identity check had become a Rust value comparison. At [OpenTouchDeck commit 4968074, `SetupSession::submit_orientation`](https://github.com/Duresa7/OpenTouchDeck/blob/49680742e3e1d429eee97a12b8bded90d30d9253/app/crates/td-companion/src/setup.rs), setup compared notices before and after the operation. A new TypeScript object would differ under `!==` even with identical fields; equal Rust notice values did not. The second identical failure therefore advanced setup to Done while the Device kept its old orientation.

On 2026-11-13, [fix commit af304d7 in PR #387](https://github.com/Duresa7/OpenTouchDeck/commit/af304d777f80b07943a2b93a37933b93cd538822) replaced the inference with explicit outcomes from `confirm_orientation` and `restart`. Setup used that outcome to decide whether to advance and used the notice only to explain a failure. The fix was on the PR branch, not a claim that M2 had merged.

On 2026-11-13, the same [fix commit's session tests](https://github.com/Duresa7/OpenTouchDeck/blob/af304d777f80b07943a2b93a37933b93cd538822/app/crates/td-companion/src/devices/sessions_tests.rs) added `setup_stays_on_orientation_through_the_same_failure_twice`. It exercised two identical failures, checked that setup stayed on Orientation and then retried successfully.

Related: [coding standards](../../CODING_STANDARDS.md#operation-results-and-concurrency), [2.0 plan](native-app-plan.md).
