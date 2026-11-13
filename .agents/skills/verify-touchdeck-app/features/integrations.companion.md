# integrations.companion

Spec: `app/ui/e2e/integrations.companion.spec.ts`. Real Companion with mock Devices.

Run `$TA ui-e2e integrations.companion --companion --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- CLIProxyAPI is set up against the local hub, and its key shows only as Set
- moving CLIProxyAPI to another origin makes the Companion forget the key, and the view says so
- the hub never sees a usage-queue, reset-quota or credit-consume call
- ${r.path ?? ''} ${r.body?.url ?? ''}

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
