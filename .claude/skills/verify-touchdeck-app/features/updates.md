# updates

Spec: `app/ui/e2e/updates.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e updates --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- is not rendered when no update exists, and About says up to date
- shows a dot when an update exists, opens on mouse hover without moving the nav, and closes on leave
- stops expanding after Later, keeps its dot, and a tooltip names it
- turns into a progress ring while downloading, then Ready to restart
- offers the version with its changes as plain text, and nothing downloads before Update
- says when no release notes were published
- shows markup in release notes as text
- downloads with progress in percent and megabytes, then Restart restarts to install
- shows kilobytes when the download size is unknown
- a download that ends ${outcome.state} ${'reason' in outcome ? outcome.reason : ''} says so, and Retry tries again
- Restart waits for a busy Device by name, and an installer that will not start can be retried
- an offline check shows no icon, says offline, and Check for updates finds the update
- a failed check says the update server is unreachable
- a source build without a public key offers no check
- Tab opens the icon at once with a visible ring, Enter lands on the first control, Escape returns focus
- opened with the pointer, the dialog itself holds focus
- focus stays in the dialog when Update replaces its buttons
- with reduced motion the icon jumps open on hover
- the update flow runs without console errors or warnings
- the icon
- keyboard focus on the icon
- the dialog at each step
- a download refused by the signature check

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
