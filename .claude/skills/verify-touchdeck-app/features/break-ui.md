# break-ui

Spec: `app/ui/e2e/break-ui.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e break-ui --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- home with ten, one and no Devices, and every one offline
- the editor with the full Layout, every Page and the inspector
- the editor for a Device with no Layout and for one with old Firmware
- integrations, each detail and the accounts
- settings, paired browsers and troubleshooting
- device setup for a new Device
- the Update icon and dialog with the longest release

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
