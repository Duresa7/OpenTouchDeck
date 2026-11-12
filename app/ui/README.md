# Open TouchDeck UI

The React UI the Companion serves. It talks to the Companion only through the generated client in
`src/lib/companion/api.gen.ts`.

Run these from `app/`:

| Command | What it does |
| --- | --- |
| `pnpm dev:ui` | The UI on `http://127.0.0.1:5174/`, against the Companion running on this PC. Start the Companion first, for example with `cargo run -p td-companion --features test-hooks -- --mock-devices 1`. |
| `pnpm dev:ui:demo` | The UI against a fake Companion with three demo Devices. No Companion needed. Add `?update=available` for an app update to offer; `none`, `no-notes`, `disabled`, `offline` and `failed` give the updater's other answers. Two browsers start paired; `?browsers=0` starts with none. `?data=worst` swaps in the worst-case data from `src/lib/companion/fake/fake-worst.ts`: names at their limits, every script, huge counts and 10 Devices. `?devices=` sets how many Devices there are, 0 to 5 on demo data and 0 to 10 on the worst case, and `?offline=all` unplugs them all. A switch at the bottom left, beside the nav rail, flips between these; `?toggle=0` hides it. |
| `pnpm build:ui` | The production build in `ui/dist`, which the Companion embeds. |
| `pnpm test:ui` | Vitest. `pnpm test` runs it too. |
| `pnpm e2e:ui` | Playwright against `dev:ui:demo`. `e2e/break-ui.spec.ts` checks every screen on the worst-case data at 800 × 600, 1180 × 760 and 3840 × 2160 in both themes. Set `TD_BREAK_SURVEY` to a folder to record each screen's screenshot and spills instead of failing. |
| `pnpm e2e:ui:companion` | One Playwright check against the real Companion with one mock Device. It builds and starts the Companion itself, with its own data folder under `target/`. |

In `pnpm dev:ui`, Vite proxies `/api`, the events WebSocket included, to the Companion, and rewrites
`Host` and `Origin` to `127.0.0.1:PORT`. It finds the port and pairing token in the Companion's
`companion.json`, and plays the part Electron main plays later: it checks the Companion's proof and
mints a window session for the page. Only the dev page itself can use the proxy or get a session. See
[`dev/companion.ts`](dev/companion.ts).

### Screenshots

The Playwright specs take screenshots only when `TD_SHOTS` names a folder. Without it the screenshot steps skip. Set it to a folder, which the specs create, and run a spec. Each shot lands in that folder under the name the spec gives it, for example `home-3-light.png`:

```powershell
$env:TD_SHOTS = "$PWD/../../.verify/shots"
pnpm e2e:ui e2e/home.spec.ts
```

`.verify/` is git-ignored, so keep shots there. `TD_BREAK_SURVEY` is the separate switch for the worst-case survey above.

### Preview fonts

`src/lib/deck/fonts.extract.mjs` generates `src/lib/deck/fonts.data.ts`, the glyph metrics the Layout preview uses to measure Tile labels. It reads the six Archivo Firmware fonts from `firmware/components/ui/fonts` (`ui_font_medium_44`, `_26` and `_22`, `ui_font_semibold_32` and `_40`, `ui_font_nameplate_26`) and LVGL's `lv_font_montserrat_48.c`. Only the Montserrat 48 file comes from the LVGL path. It is a one-off script, not part of any build. Run it from `app/ui` with the path to LVGL 9's `src/font` folder, then commit the new `fonts.data.ts`:

```powershell
node src/lib/deck/fonts.extract.mjs <lvgl>/src/font
```
