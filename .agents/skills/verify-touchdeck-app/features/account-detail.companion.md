# account-detail.companion

Spec: `app/ui/e2e/account-detail.companion.spec.ts`. Real Companion with mock Devices.

Run `$TA ui-e2e account-detail.companion --companion --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- one mock Device and a local hub expose account detail without leaking credentials

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
