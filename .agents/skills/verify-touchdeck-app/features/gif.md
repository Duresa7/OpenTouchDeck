# gif

Spec: `app/ui/e2e/gif.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e gif --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- Add Tile offers GIF only to a Device that plays GIFs
- a GIF Tile prepares, plays, refits and is sent; a second GIF waits its turn and can be cancelled
- a GIF that fails says why, keeps the raw reason under Details, and retries
- a Page with more GIF Tiles than the Device plays is refused when sent, and the Device keeps its Layout
- with reduced motion the editor shows each GIF paused on its first frame

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
