# Updater staging after shutdown

On 2026-11-13, [PR #389](https://github.com/Duresa7/OpenTouchDeck/pull/389) fixed the shutdown work tracked by [#385](https://github.com/Duresa7/OpenTouchDeck/issues/385). A download blocked after writing part of an MSI could outlast the two-second shutdown limit. Cancelling the async wrapper did not stop its blocking worker, and process exit could leave staging in the temporary directory before `TempDir` cleanup ran.

On 2026-11-13, the fix merged at [OpenTouchDeck commit 2a14ad8](https://github.com/Duresa7/OpenTouchDeck/commit/2a14ad8d2562f360deb2cb83a07e57930c253034). The [updater](https://github.com/Duresa7/OpenTouchDeck/blob/2a14ad8d2562f360deb2cb83a07e57930c253034/app/crates/td-updater/src/lib.rs) checked the Companion stop signal around requests, reads and writes, discarded cancelled results and limited each network read to 10 seconds. The read timeout still exceeded the shutdown budget, so it could not guarantee cleanup before exit.

On 2026-11-13, [PR #389](https://github.com/Duresa7/OpenTouchDeck/pull/389) also added a startup sweep after the single-instance lock and before the first poll. At [OpenTouchDeck commit 2a14ad8](https://github.com/Duresa7/OpenTouchDeck/blob/2a14ad8d2562f360deb2cb83a07e57930c253034/app/crates/td-updater/src/lib.rs), it removed only real directories with the expected staging prefix and suffix whose contents were regular MSI or signature files with expected names. It left links, nested directories and unrelated files untouched. This recovered staging left by interrupted downloads and by installer handoff.

On 2026-11-13, the [regression tests at that commit](https://github.com/Duresa7/OpenTouchDeck/blob/2a14ad8d2562f360deb2cb83a07e57930c253034/app/crates/td-updater/src/tests.rs) held a body read with channels, signalled shutdown, swept with a new updater and only then released the old worker. They checked both removal of owned staging and preservation of foreign content.

Related: [worker and persistence standards](../../CODING_STANDARDS.md#persistence-and-worker-lifetime), [review and merge gates](review-and-merge.md).
