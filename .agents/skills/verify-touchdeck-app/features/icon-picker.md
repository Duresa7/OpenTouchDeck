# icon-picker

Spec: `app/ui/e2e/icon-picker.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e icon-picker --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- typing fast sends one search, and a picked logo becomes the Tile’s Asset
- recent picks come first, newest first
- a recent that no longer resolves drops out quietly
- uploading an image still sets the Tile’s icon
- a render failure explains itself and retries
- an offline Companion, no results and an empty search each say what to do
- works from the keyboard alone
- closing aborts the search in flight, and a closed picker runs no timers

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
