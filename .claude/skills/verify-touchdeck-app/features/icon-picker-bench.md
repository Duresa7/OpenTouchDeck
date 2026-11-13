# icon-picker-bench

Spec: `app/ui/e2e/icon-picker-bench.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e icon-picker-bench --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- first open after a page load, and repeat opens

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
