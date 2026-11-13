# Layouts on the deck

From M2-04 on, the deck draws its pages, tiles, icons, and actions from a layout file in the format of `schema/README.md`. The firmware embeds `schema/examples/builtin.json` as the built-in layout. At boot it loads `/storage/layouts/<orientation>.json`, and a missing or invalid file falls back to the built-in layout with `layout: <reason>, using builtin` in the log. Every load logs `layout: loaded <source> <name> pages=<n> tiles=<n> icons=<loaded>/<wanted> in <ms> ms`.

## Commands

```
layout state            @td {"cmd":"layout","ok":true,"op":"state","source":"builtin","name":"Built-in","orientation":"landscape","pages":2,"tiles":22,"icons_loaded":0,"icons_missing":0,"bytes":0,"load_ms":N,"screen_events":4,"timers":N,"error":""}
layout reload           @td {"cmd":"layout","ok":true,"op":"reload","source":"storage","name":"Development","pages":4,"tiles":13,"icons_loaded":0,"icons_missing":0,"load_ms":N}
layout use builtin      @td {"cmd":"layout","ok":true,"op":"use","source":"builtin",...}
layout use storage      the same as reload
```

- `source` is `storage` or `builtin`. `bytes` is the stored file's size, 0 for the built-in layout. `load_ms` runs from the start of the read to the end of the rebuild.
- `error` is empty after a load that worked. After a refused file it is `<rule key>: <message>`, with the same key `node schema/check-layouts.mjs` prints, such as `tile.overlap: page "p" tiles "a" and "b" overlap`. The loader adds `file.missing`, `file.read`, `display.busy`, and `internal.no-memory`.
- A reload whose file is refused answers `ok` false with that error, and the layout on screen and the rest of `layout state` stay as they were. At boot a refused file falls back to the built-in layout, so `source` is `builtin` with the reason in `error`.
- `screen_events` counts the event callbacks on the screen, 4, and `timers` counts LVGL's timers. Both stay flat across reloads.
- A malformed line answers `usage: layout state | reload | use builtin|storage`. Before the display starts every form answers `display not ready`.
- `ui tile <id>` adds `action`, the schema's type name, and `icon`, the layout's string or empty.
- A `key` tile with `mods` sends `kbd (mods | active) [usage]` and `kbd 0 []`. A `page` tile slides to its page. A `launch`, `open`, `command`, or `widget` tile logs `ui: press <page>/<tile> <kind>` and sends nothing.

## Recipes

Use absolute paths, and run `$TD serial cmd "power idle-timeout 86400"` first so the screen stays lit.

- **Built-in on empty storage.** Run `$TD serial cmd "store rm /storage/layouts/landscape.json"`, then `$TD serial cmd "layout use builtin"` and `$TD serial cmd "layout state" > $E/state.json`. `source` is `builtin` with 2 pages and 22 tiles.
- **dev.json from storage.** Compact it first, `node -e "process.stdout.write(JSON.stringify(JSON.parse(require('fs').readFileSync('schema/examples/dev.json','utf8'))))" > $E/dev.json`. Then run `$TD serial store put /storage/layouts/landscape.json --from $E/dev.json`, `$TD serial cmd "layout reload"`, and `$TD serial cmd "layout state" > $E/dev-state.json`. `source` is `storage`, with 4 pages and 13 tiles. `$TD serial cmd "ui tile chord"` gives `action` `key`, and `ui goto <n>` with `$TD serial screenshot` shows each page.
- **A chord.** On dev.json's first page, run `$TD serial cmd "keys record start"`, `$TD serial cmd "ui tile chord"`, `$TD serial cmd "touch tap <cx> <cy>"`, `$TD serial cmd "keys record stop"`, and `$TD serial keys --out $E/chord.json`. `brief` is `kbd 3 [104]`, `kbd 0 []`.
- **A refused reload.** With dev.json loaded, put a layout with an overlap at the same path and run `layout reload`. It answers `ok` false with `tile.overlap: ...`. `layout state` still reports dev.json, with the overlap in `error`.
- **An icon.** Put a `TDI1` file, such as `schema/fixtures/icon-64.tdi`, at `/storage/icons/<16 hex>.bin` and a layout whose tile names `file:<16 hex>`. After `layout reload`, `icons_loaded` is 1 and the tile shows the 64 by 64 image above its label.

## Gotchas

- `store put` writes the stored file only. The deck reads it at the next boot or `layout reload`.
- The deck refuses a stored file over 32768 bytes before parsing it. A pretty-printed layout can pass the validator and still be refused, so always put the compact form.
- A reload deletes and rebuilds every tile object, so a tap aimed during a reload can land on nothing. Wait for the reply first.
