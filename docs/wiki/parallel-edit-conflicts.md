# Sequencing branches that edit the same files

On 2026-11-13, the work in [#376](https://github.com/Duresa7/OpenTouchDeck/issues/376) waited on [PR #384](https://github.com/Duresa7/OpenTouchDeck/pull/384) because both touched `app/crates/td-core/src/editing.rs` and `docs/layout-editing.md`. The follow-ups concerned schema recomputation, input toggles, saved rejections and an identical receive during a send. Sequencing the work avoided competing edits to the same editing contract.

On 2026-11-13, [PR #384's implementation report](https://github.com/Duresa7/OpenTouchDeck/pull/384) listed those #376 follow-ups as addressed within its editor-session work. That report did not establish that the PR or issue had closed; the dependency belongs to this dated history rather than a permanent instruction to wait.

On 2026-11-13, the [M4 plan](https://github.com/Duresa7/OpenTouchDeck/issues/356#issuecomment-6845285016) applied the same lesson. [#393](https://github.com/Duresa7/OpenTouchDeck/issues/393) split M2 screen files before further screen work, and [#395](https://github.com/Duresa7/OpenTouchDeck/issues/395) reserved page slots so Icon packs and Brandfetch could use separate files. [#398](https://github.com/Duresa7/OpenTouchDeck/issues/398) and [#399](https://github.com/Duresa7/OpenTouchDeck/issues/399) explicitly required sequencing because both edited `settings.slint`.

Related: [M4 ticket sequence](m4-design.md#ticket-sequence), [review follow-ups](review-and-merge.md).
