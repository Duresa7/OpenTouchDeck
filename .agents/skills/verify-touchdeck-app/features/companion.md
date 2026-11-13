# companion

Spec: `app/ui/e2e/companion.spec.ts`. Real Companion with mock Devices.

Run `$TA ui-e2e companion --companion --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- Home lists the mock Devices from the real Companion and opens one
- a Layout pushed to a Device shows on its miniature, and stays when it is unplugged
- the Layout screen sends a Layout to a mock Device, and the Device shows it
- a send after the mock Device changed offers Keep mine, and Load from replaces after a confirm
- the Device panel renames a mock Device, switches its orientation with a restart, and restarts it
- the Device panel turns on the photo frame of a mock Device, which stores it
- a pressed command Tile waits for approval on this PC with its arguments escaped, and clears when the dialog answers
- Always allow lists the action, the Action log reads and follows the presses, and Remove makes the next press ask
- Key test shows the mock Device's key reports from the key-mirror topic, suppressed while the dialog waits
- a board on a mock port shows a New board card that waits for a click, and leaves when unplugged
- a name saved through settings_patch ends setup, and a ready Device never opens it
- a New board installs once this PC accepts, comes up needing setup, and setup makes it ready
- a declined install writes nothing, keeps the New board card, and opens no setup

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
