# new-board

Spec: `app/ui/e2e/new-board.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e new-board --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- a New board installs only after the click, with progress, then sets up as a Device
- the installed card says what comes next while the board restarts
- a declined install is explained and leaves the board as it was
- a failed install shows Details, and Retry asks again
- Ignore folds the board into a chip that brings it back
- an unplugged board loses its card
- focus is visible on the card buttons

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
