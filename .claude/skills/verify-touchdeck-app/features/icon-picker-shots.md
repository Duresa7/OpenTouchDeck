# icon-picker-shots

Spec: `app/ui/e2e/icon-picker-shots.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e icon-picker-shots --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- recent picks and built-in icons
- logo results
- a render failure
- the Companion offline
- worst-case names

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
