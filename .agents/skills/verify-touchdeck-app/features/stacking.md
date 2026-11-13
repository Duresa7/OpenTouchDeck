# stacking

Spec: `app/ui/e2e/stacking.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e stacking --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- Settings
- the Update dialog
- the Replace confirm
- a confirm opened from Settings draws over Settings and its scrim covers it
- the nav dock draws over the canvas
- the Layout menu draws over the editor
- a tooltip draws over the editor
- Home
- the Layout editor
- Settings open
- the Replace confirm

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
