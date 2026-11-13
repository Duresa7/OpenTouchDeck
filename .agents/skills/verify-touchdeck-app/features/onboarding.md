# onboarding

Spec: `app/ui/e2e/onboarding.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e onboarding --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- an unknown Device sets up once, and its name shows on Home and in the editor
- Skip leaves the Device usable, does not reopen, and the Device panel can still name it
- the Close button skips the same way
- a name the Device would refuse stays on the step with the reason
- Starter sends the Starter Layout, and Edit Layout opens it
- Blank sends one empty Page
- Keep leaves the Layout already on the Device
- a new orientation restarts the Device before the Layout step
- updates the Device
- can be skipped
- a rollback says the Device still works, and Skip still moves on
- a v1 Device updates first, then takes its name
- a skipped v1 Device says naming needs newer Firmware and moves on
- focus is visible on the name and on each choice

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
