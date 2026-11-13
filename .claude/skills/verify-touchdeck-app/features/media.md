# media

Spec: `app/ui/e2e/media.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e media --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- a Volume bar from Add Tile reaches the Device as a bar a drag can set
- a bar turned back into its value drops the drag
- a Now playing Tile reaches the Device with the title, artist, cover and state streams
- without the Firmware, Volume and Now playing Presets are placed anyway and say Needs Firmware
- the Integrations page lists Volume and Media with their Preset tiles
- Volume and Now playing Tiles in the editor and the deck preview
- Volume and Media rows on the Integrations page

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
