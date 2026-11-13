# home

Spec: `app/ui/e2e/home.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e home --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- ${count} Devices keep their size and wrap with the design gap
- an unplugged Device keeps its Page and shows last seen
- a pushed Layout reaches the miniature without reloading
- ${method} opens the selected Device Layout
- keyboard focus is visible; fine-pointer lift stops with reduced motion
- proof ${count} Devices ${theme}

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
