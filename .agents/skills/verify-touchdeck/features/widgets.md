# Widgets, dashboards and offline dimming (M2-06)

The app fills widget tiles with `widget.set {tile, value, caption, bar, stale}` and dashboard pages with
`dashboard.set {page, rows:[{label, value, bar}]}`. `bar` is a fraction from 0 to 1, or -1 or absent for none. The
deck keeps up to 24 bytes of value, 16 of caption, and 8 dashboard rows of 16-byte label and value, and clamps a bar
above 1 to 1. A redraw timer draws dirty tiles every 100 ms, so a burst of sets costs one redraw. While the link is
offline, every `launch`, `open`, `command` and `widget` tile draws at 40 percent opacity, and a tap on one logs
`ui: offline <tile>` and sends nothing. Key, mod, consumer and page tiles never dim.

## Sub-features

- `widget-tile` draws the value in the tile font, the caption under it (the tile's label until the app sends one), and
  a 6 px glow bar 12 px above the tile's bottom edge, as wide as the tile times `bar`. `stale` draws the text at
  60 percent.
- `dashboard-page` draws up to 8 rows of label, value and bar in the grid area. Rows the app has not sent stay hidden.
- `offline-dim` follows the link's `online` flag, which `hello` sets and 5 s of silence clears.

## How to get to it (user POV)

- `$TD serial cmd "ui state"` reports `online` and `dimmed`, the count of tiles drawn dim now.
- `$TD serial cmd "ui widget <tile>"` replies with `value`, `caption`, `bar_pct` (whole percent, -1 for none),
  `stale` and `updated_ms`. `unknown tile` for a tile that is not a widget tile of the loaded layout.
- `$TD serial cmd "ui dashboard <page>"` replies with `rows`, `labels`, `values`, `bars_pct` and `updated_ms`.
- `$TD link hold --seconds 10 [--shot <png>]` says hello and pings every second, so the deck is online for the hold.
- `$TD link widget-set '<json>' [--seconds N] [--shot <png>]` and `$TD link dashboard-set '<json>' ...` say hello,
  send the message, print both replies, and hold the link online for N seconds. `--shot` takes one screenshot
  `--shot-at` seconds (default 1.5) after hello on the same port, while the link is online. Pass the JSON without `m`.

## Driving it with touchdeck.ps1

Load `schema/examples/dev.json` first. Its widget tiles are `context`, `cost`, `model` and `proxy` on page 2
(`usage`), its app tiles `launch`, `open` and `command` on page 1, and its dashboard page is `accounts`.

```powershell
$TD serial cmd "ui goto 2"
$TD serial screenshot --out $E\offline.png                       # widget tiles dim
$TD link hold --seconds 4 --shot $E\online.png                   # widget tiles bright
$TD link widget-set '{"tile":"context","value":"42%","caption":"context","bar":0.42}' --shot $E\widget.png
$TD serial cmd "ui widget context"
```

In PowerShell, pass the JSON in single quotes. A refusal (`unknown tile`, `too many rows`) prints `ok:false` and
exits 1.
