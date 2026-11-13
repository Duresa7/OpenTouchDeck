# Coding standards

Read these rules before implementing or reviewing a change. Apply each relevant rule to the changed code; cite the rule and the affected behaviour when reporting a violation. Existing code examples explain conventions, not blanket compliance. Historical findings live in the [wiki](docs/wiki/index.md); build commands live in [app development](app/README.md) and [Firmware development](docs/firmware.md).

## Operation results and concurrency

- Preserve TypeScript identity semantics when porting to Rust. Replace notice-identity checks with an explicit operation result; never substitute structural equality to decide success. Equal error text can describe two separate failures. Example: [#387's fix af304d7](https://github.com/Duresa7/OpenTouchDeck/commit/af304d777f80b07943a2b93a37933b93cd538822).
- Keep blocking Device, registry, disk, dialog and network I/O off the Slint UI thread. Use the runtime for async work and `spawn_blocking` or the existing Device actors for synchronous work; post view updates through the event loop. Blocking callbacks freeze input and accessibility. Examples: [M4 requirements #393](https://github.com/Duresa7/OpenTouchDeck/issues/393), [Device actors](app/crates/td-companion/src/devices/mod.rs) and [event-loop dispatch](app/crates/td-app/src/lib.rs).
- Serialize competing writes through the operation owner, including persistence and publication of the result. Test a quick double toggle; independent tasks may finish in the opposite order. Example: [per-Device settings gates](app/crates/td-companion/src/devices/operations.rs).

## Persistence and worker lifetime

- For settings updates, read and validate the existing document, change only owned fields, preserve unknown fields and replace the file atomically through `td_core::app_dirs::write_atomic`. Report damage without rewriting the damaged file. Atomic replacement prevents partial writes; preserving fields and refusing damage protects user data. Example: [the update preference implementation in #387](https://github.com/Duresa7/OpenTouchDeck/blob/af304d777f80b07943a2b93a37933b93cd538822/app/crates/td-core/src/settings.rs).
- Give long-running workers a stop signal and clean up their owned side effects on success, failure and cancellation. Bound blocking calls; if process exit can precede cleanup, recover owned leftovers at startup. Cancelling an async wrapper does not stop `spawn_blocking`. Example: [updater fix #389](https://github.com/Duresa7/OpenTouchDeck/pull/389).

## Secrets

- Compare secret bytes and MACs with a constant-time primitive, including truncated MACs. Ordinary equality may reveal the matching prefix through timing. Example: [`subtle::ConstantTimeEq` in net-v2](app/crates/td-link/src/net/wire.rs).
- Keep key material out of logs, error messages, UI views and `Debug` output. Implement redacted formatting for secret-bearing types rather than deriving `Debug`. Diagnostics must not become a second credential store. Example: [`Credential`'s custom `Debug`](app/crates/td-link/src/net/mod.rs).
- A claim that the Device sends no stable identifier covers every emitter on that radio or transport, including discovery, the keyboard role, the GAP Device Name and mDNS. The PR lists each emitter and its address type.

## Module boundaries and errors

- Keep pure Layout rules in `td-core`, Device and workflow policy in the Companion, and Slint code responsible for rendering and dispatch. Exercise policy through the same typed interface the UI calls. This keeps behaviour testable without a window. Examples: [Layout editing](docs/layout-editing.md) and [M4 sessions #394](https://github.com/Duresa7/OpenTouchDeck/issues/394).
- Expose a module's intended API through its root and keep implementation modules private. Call the exported operation rather than reaching into its storage or actor. This lets internals change without changing callers. Example: [`devices/mod.rs` re-exports](app/crates/td-companion/src/devices/mod.rs).
- Return typed errors for conditions callers must distinguish; preserve unknown outcomes and partial successes instead of flattening them into a generic string. Convert to display text at the adapter. A lost reply does not prove the Device rejected a write. Examples: [`DeviceError`](app/crates/td-companion/src/devices/error.rs) and [`SettingsOutcome`](app/crates/td-companion/src/devices/settings.rs).
- Use the [glossary](GLOSSARY.md) for domain names and preserve established wire names unless the protocol change explicitly requires migration. Shared names prevent UI, tests and Firmware from describing different concepts. Example: [ADR 0003](docs/adr/0003-brand-changes-wire-names-stay.md).

## Tests and review

- Include a regression test with every bug fix. Demonstrate that it fails with the fix removed and passes with the fix applied. A passing test alone may never have exercised the defect. Example: [#386's failure proofs](https://github.com/Duresa7/OpenTouchDeck/pull/386).
- Put private-unit tests in a `#[cfg(test)]` module beside the implementation or a sibling `tests.rs`; put public-contract tests in the crate's `tests/` directory. Keep mock hooks behind their test feature or test configuration. This tests the intended interface without shipping test controls. Examples: [updater unit tests](app/crates/td-updater/src/tests.rs), [Companion contract tests](app/crates/td-companion/tests/companion.rs) and [test-hook exports](app/crates/td-companion/src/lib.rs).
- Coordinate concurrency tests with channels or checkpoints that hold the contested operation, then assert the observable result and cleanup. Timing sleeps alone do not prove the race occurred. Example: [#389's staged-read regression](https://github.com/Duresa7/OpenTouchDeck/pull/389).
- Test a timeout, deadline or shutdown budget with an operation that never returns; assert the bound and the cleanup.
- Before a merge, inspect every check on the latest head and wait for all scheduled checks to finish successfully, including non-required checks. Account for intentional skips and neutral reporting checks; treat failures, cancellation and pending work as unresolved. The required gate can omit other workflows. Example: [#386's merge history](docs/wiki/review-and-merge.md).
- Triage review findings against the intended behaviour. Fix blockers in the PR and log non-blocking improvements as follow-up issues with evidence and a reason for deferral. This preserves useful findings without expanding every patch. Examples: [#376](https://github.com/Duresa7/OpenTouchDeck/issues/376) and [#401](https://github.com/Duresa7/OpenTouchDeck/issues/401).
