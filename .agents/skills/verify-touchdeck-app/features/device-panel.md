# device-panel

Spec: `app/ui/e2e/device-panel.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e device-panel --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- renames the Device, and the new name shows in the header and on Home
- refuses a blank name with the rule and keeps the old one
- switches orientation after the restart confirm, and the Device restarts
- Cancel keeps the orientation and restarts nothing
- sets brightness from the slider and shows the value
- Restart restarts the Device and its status comes back
- updates the Firmware with progress, then shows the new version
- an update that rolls back says so, and keeps the old version
- a cancelled update says it stopped, keeps the old version, and offers Update again
- a v1 Device's name and brightness say they need newer Firmware
- schema 1 Tiles carry no Needs Firmware badge, even on v1 Firmware
- brightness on Firmware that can't set it explains why
- turns the photo frame on, sets when it starts and picks its folder
- in a browser the folder shows as text, and choosing one is left to the app
- the photo frame on Firmware without it explains why
- Enter opens the panel with focus on the first field, and Escape returns focus to the name
- every control in the panel is reachable with Tab and shows the focus ring
- the open panel
- an update in progress
- the photo frame, on with a folder
- the photo frame in a browser
- focus on the Device name

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
