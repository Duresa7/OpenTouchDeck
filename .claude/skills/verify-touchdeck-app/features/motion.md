# motion

Spec: `app/ui/e2e/motion.spec.ts`. Fake Companion; proves rendered UI and client behavior.

Run `$TA ui-e2e motion --out <absolute fresh evidence directory>`. Require exit 0 and inspect `report.json` for passed, skipped and failed tests. Screenshots are under `results/` and any spec-defined TD_SHOTS paths.

Observable checks (parameterized titles retain their source placeholders):

- the Update icon closes from where it is when the pointer leaves mid-open, and reopens from there
- Escape mid-open shrinks Settings away from where it is, then it opens fully
- a second click mid-open closes the Device panel from where it is, and its chevron turns back
- a tooltip fades back from where it is when the pointer leaves mid-open
- a Device on Home lifts by its transform and a shadow crossfade, and settles back from where it is
- pressing a Device on Home scales it to 0.985, and letting go turns it back from where it is
- a Device's screen lights up as it connects and dims as it goes, turning back from where it is
- a switch thumb slides back from where it is when clicked again mid-slide
- pressing a button scales it to 0.97 and letting go turns it back
- a dropped Tile settles into its cell from where it was let go
- download progress fills to where the download is
- the try-it Page slide ends on the new Page, and a press mid-slide stops it on that Page as the Device does
- the try-it Page slide and a Page tab switch keep every frame
- Settings fades in without growing
- the Device panel fades in without growing, and its chevron turns at once
- a tooltip fades in without growing
- the Update icon jumps open and shut
- a Device on Home neither lifts nor shrinks under the pointer
- a Device's screen still fades as it connects
- a switch thumb and a dropped Tile jump to their end state
- download progress steps without easing
- the try-it Page changes at once, with no slide

Mock and browser proof does not establish native dialogs, real Device behavior or Electron window behavior. Shot and benchmark specs may require their documented environment switches; inspect skips in the JSON report.
