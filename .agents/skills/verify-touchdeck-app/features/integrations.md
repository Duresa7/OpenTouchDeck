# integrations

Spec: `app/ui/e2e/integrations.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e integrations --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- lists CLIProxyAPI with its Preset tiles and a live status light
- a fresh install offers Set up, which opens the form with focus on its first control
- setting up with a key shows it only as Set, and never keeps it in the page
- checks fields inline and warns before a key travels unencrypted
- the status light follows the hub and the switch
- Check now asks the hub once, then waits out the floor politely
- moving the hub to another origin forgets the key and says so
- Forget asks first, then removes the key
- shows every account with its limits, as the AI usage page did
- Add to Layout refuses a full Page, then places a Preset tile in the first free cell
- Add Tile lists Preset tiles by Integration and places one where it points
- Add Tile points to Integrations when none offers Preset tiles
- focus is visible on the list and on every control of the CLIProxyAPI view

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
