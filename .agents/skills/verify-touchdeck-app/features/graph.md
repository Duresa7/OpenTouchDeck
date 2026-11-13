# graph

Spec: `app/ui/e2e/graph.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e graph --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- Graph waits for Firmware that draws graphs, then binds a Tile to any numeric stream
- the preview keeps as many samples as the Device does
- a This PC Preset placed from Add Tile is a graph of its stream
- without graph Firmware a This PC Preset stays a value Tile
- an offline Device shows its graphs stale, at 60% with the unavailable marker
- graph Tiles, one picked

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
