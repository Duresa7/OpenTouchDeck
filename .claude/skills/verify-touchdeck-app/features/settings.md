# settings

Spec: `app/ui/e2e/settings.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e settings --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- toggles and is still set when Settings opens again
- lists each browser with its last use, and Forget asks first, then removes it for good
- Forget all removes every browser and shows how to pair one
- Reset pairing confirms first, then signs out every browser while this window carries on
- with none paired it says how to pair one
- shows each Device with its link and Firmware fields, and Back returns to Settings
- Connection help on an empty Home opens Troubleshooting, and the nav opens Settings again
- a console line round-trips, and the console listens only while it is open
- a line off the allowlist is not sent, and the console lists what it takes
- Update from a file updates a Device from the chosen file, and a file that is not Firmware says so
- shows the version and build, and Check for updates shows Checking…, then Up to date
- Tab reaches every Settings control with the visible focus ring
- Escape closes Settings from Troubleshooting too

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
