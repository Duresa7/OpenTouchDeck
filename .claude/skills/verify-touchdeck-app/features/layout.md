# layout

Spec: `app/ui/e2e/layout.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e layout --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- imports dev.json with a tab per Page and the first Page drawn
- moves F5 onto a free cell, then refuses a move onto Mute and keeps F5 in place
- sends and shows the Device's Page and Tile counts
- grows Unused to 2 by 2, then refuses a size past the grid edge
- loads the Device's Layout after a confirm, and every Tile matches
- exports a file that passes the Layout rules
- renders a 64-Tile Page and times five sends
- keeps a separate Layout for portrait, and both outlive a reload
- tries Pages out like the Device: swipe, tap, a page jump and the arrow keys
- explains a refused change and a file it cannot open
- holds up to the worst realistic Layout
- a send that finds the Layout changed on the Device offers Replace and Keep mine
- Add Tile places a Tile with its kind and selects it; Cancel and Escape close it
- the keyboard moves, resizes, clears and deselects a Tile
- a Device that is not connected can still be edited, but not sent to
- focus is visible on Tiles, Page tabs and header controls

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
