# Project wiki

Historical notes seeded on 2026-11-13. Each entry records what a decision or source established on its date; consult the linked issue or code before relying on it for new work.

When adding history, date each claim and cite its issue or PR. Pin code claims to a commit in this repository. Append later decisions and mark earlier claims superseded instead of rewriting them. Keep one topic per page, link related pages, and add every page to this index. Cite sources rather than copying them.

## Native app

- [2.0 plan](native-app-plan.md): The architecture decision, changed cutover timing, milestone scopes and M1 closure.
- [M4 design](m4-design.md): Typed sessions, Device panel scope and the eight implementation tickets.

## Recurring problems

- [TypeScript identity during Rust ports](typescript-port-outcomes.md): Repeated identical failures need an explicit operation result.
- [Updater shutdown cleanup](updater-shutdown-cleanup.md): A stalled read can outlive shutdown and require cleanup on the next start.
- [Bluetooth keyboard privacy](ble-keyboard-privacy.md): Private discovery hints do not hide a keyboard role's public address.
- [Cryptographic test fixtures](test-crypto-alerts.md): Test-only nonces triggered CodeQL alerts and received scoped dismissals.
- [Overlapping branches](parallel-edit-conflicts.md): Shared editing files and Device settings screens needed sequenced work.

## Review process

- [Review and merge gates](review-and-merge.md): Wait for all checks and separate blockers from follow-ups.

For implementation and review rules, read [coding standards](../../CODING_STANDARDS.md). For domain vocabulary and architectural decisions, read the [glossary](../../GLOSSARY.md) and [ADRs](../adr/).
