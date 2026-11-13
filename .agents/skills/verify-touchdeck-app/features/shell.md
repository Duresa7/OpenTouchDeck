# shell

Spec: `app/ui/e2e/shell.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e shell --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- the window is titled Open TouchDeck and opens on Home with the Devices and their status
- the shell runs without console errors or warnings
- each destination is one click from the cluster
- a tooltip names each button on hover
- Settings opens over the current screen, Escape closes it, and a change is still there on reopen
- About shows the version and build the Companion reports
- light and dark follow the Windows setting
- forcing a theme with Appearance sets data-theme over the Windows setting
- the dialog grows into place by default
- with reduced motion the dialog appears without moving
- Tab reaches every nav button with the visible focus ring

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
