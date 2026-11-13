# inspector

Spec: `app/ui/e2e/inspector.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e inspector --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- sets up and sends every action kind, keeping the program across an app and a command
- explains an invalid value next to its field, and Send to Device refuses until it is fixed
- a pressed command Tile waits for approval on this PC, and the wait clears when the dialog answers
- the pending card writes an invisible character in an argument as an escape, and the Tile keeps the raw value
- lists an always-allowed command and removes it
- Key test shows the Device's key reports, suppressed while the dialog waits, and the keys this PC gets
- focus is visible on the inspector fields

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
