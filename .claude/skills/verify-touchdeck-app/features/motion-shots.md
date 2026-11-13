# motion-shots

Spec: `app/ui/e2e/motion-shots.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e motion-shots --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- 01 the Update icon opens
- 02 Settings opens
- 02 Settings opens with reduced motion
- 03 the Device panel opens
- 04 a tooltip appears
- 05 a Device on Home lifts under the pointer
- 06 a Device on Home is pressed
- 07 a Device's screen lights up as it connects, ${theme}
- 08 a switch is turned on
- 09 a button is pressed
- 10 a dropped Tile settles into its cell
- 13 the try-it Page slide

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
