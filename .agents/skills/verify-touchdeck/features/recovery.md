# Recovery hold

From M2-08 on, a single finger held still on the screen for 10 seconds puts the deck into recovery. The screen turns into a red `Release to reset` band. On release the deck writes the marker `/storage/recovery` and restarts 250 ms later. The next boot deletes the marker, runs the built-in pages in landscape with a `Recovery` caption, and leaves every stored file alone. Recovery stays on until the user picks the stored layout again or writes a new layout or settings.

## Sub-features

- `recovery-hold`: one contact that stays within 24 px of its first sample for 10,000 ms arms. A contact that moves past the slop is spent until it lifts and never arms later. A release before 10 s returns to idle.
- `recovery-swallow`: an armed contact never reaches the UI, so it fires no tile and sends no key, release included. A hold shorter than 10 s on a tile fires that tile normally.
- `recovery-band`: the full-screen band, sized from the current screen geometry, so it covers 1024x600 in landscape and 600x1024 in portrait. A layout rebuild while armed draws it again.
- `recovery-boot`: the marker boot. Built-in pages, landscape whatever `settings.json` says, the `Recovery` caption, and `layout`, `settings`, and icon files kept byte for byte. The stored layout is parsed only so `layout state` can report its error.
- `recovery-stay`: `layout reload` in recovery stays on the built-in pages.
- `recovery-leave`: `layout use storage`, `layout.put`, and `settings.put` turn recovery off, remove the caption, and log `recovery: mode off: <why>`. A plain `reboot` also leaves it, because the marker is already gone.
- `recovery-state`: the `recovery state` console command reports `mode`, `watcher` (`IDLE`, `HOLDING`, `ARMED`), `held_ms`, and `armed_count`. `hello` and `layout state` carry `recovery` too.
- `recovery-arm-hook`: `recovery arm` is a test hook that writes the marker and restarts at once, with no hold.

## How to get to it (user POV)

- Hold one finger still anywhere on the screen, on a tile or between tiles, for 10 seconds. The screen turns red and reads `Release to reset`. Lift, and the deck restarts on its built-in pages with `Recovery` at the top.
- The app's Recover action and the app's layout push both leave recovery, because they send `layout use storage` or `layout.put`.

## Driving it with touchdeck.ps1

Preconditions:

- Hold the board lock, flash the build under test, and run Doctor.
- Run `$TD serial cmd "power state"` first and wake the screen if it is `OFF`, as the [Idle](./idle.md) rule says. A wake touch is eaten and never starts a hold.
- For the stored-layout proof, store a real layout first with `$TD serial store put` and `layout use storage`, and keep a copy of the board's original layout and settings so you can put them back.

Commands:

- Hold. `touch swipe` is capped at 2,000 ms, so a 10 s hold is `$TD serial cmd "touch begin <x> <y>"`, a wait of about 10.5 s, then `$TD serial cmd "touch end"`. `touch begin` and `touch move` take coordinates in the current screen size, so a portrait hold may use y up to 1023. Poll `$TD serial cmd "recovery state"` during the wait. `watcher` reads `HOLDING` before 10 s and `ARMED` after.
- Band. `$TD serial screenshot` while armed shows the red band over the whole screen.
- Keys. `$TD serial keys` after the hold shows no report from the held contact. For the short-hold check, hold a tile for about 6 s and expect exactly that tile's press and release.
- Boot. After `touch end`, reopen the port in a new `$TD` call. `recovery state` reports `mode: true`, `hello` and `layout state` report `recovery: true`, the screenshot shows the built-in pages with the `Recovery` caption, and `$TD serial store ls` and `store get` show the stored files unchanged.
- Stay. `$TD serial cmd "layout reload"` keeps the built-in pages and `mode: true`.
- Leave. `$TD serial cmd "layout use storage"` brings the stored layout back. `recovery state`, `layout state`, and `hello` report false, and the caption is gone.
- Shortcut. `$TD serial cmd "recovery arm"` skips the hold, for runs that only test the boot side.

## Gotchas

- Command overhead stretches a scripted hold. A "9 s" hold has measured 10.18 s and armed, so keep a short-hold check near 6 s.
- A recovery boot runs landscape even when the stored settings say portrait. `settings.put` judges `reboot_required` against the orientation on screen, not the stored one.
- Before the cleanup PR, leaving recovery through `layout use storage` or `layout.put` did not re-apply the stored `idle_timeout_s` until `settings.put` or a reboot. Check the build under test before relying on either behavior.
- The marker is deleted at the start of the recovery boot, so a reset during recovery returns to the stored layout. That is by design.
- Screenshots and injected touches prove the UI logic and LVGL's input path, not the GT911 or the glass. Whether a real finger can hold still within 24 px for 10 s is pending operator.
