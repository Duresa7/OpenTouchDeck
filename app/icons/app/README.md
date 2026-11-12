# App and tray icons

The Open TouchDeck 2.0 icon is the Device seen from the front: a slab of black glass, a 3 × 2 Page of Tiles with the first Tile lit, and the pager under the grid. Only lit things have colour. Light comes from the top, and there is no text.

Nothing uses these files yet. The 2.0 window and tray work will embed them. The Electron installer keeps its own icon, [`app/electron/installer/icon.ico`](../../electron/installer/icon.ico).

## Files

| Path | What it is |
| --- | --- |
| `open-touchdeck.ico` | The app icon: 14 sizes, for the executable, the window and the installer |
| `tray.ico` | The notification area icon: 16, 20, 24 and 32 px |
| `svg/icon-<size>.svg` | One drawing per app size. `icon-256.svg` is the master |
| `svg/tray-32.svg` | The 32 px tray drawing |
| `png/icon-<size>.png`, `png/tray-<size>.png` | The same images as loose PNGs, for code that wants pixels instead of an `.ico` |
| `build.py` | Renders the PNGs, writes both `.ico` files and reads them back |

## Which size Windows uses

Windows takes the entry that matches the size it needs, and scales the next larger one when there is no match. `open-touchdeck.ico` holds every size in [Microsoft's Windows 11 table](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction), so nothing is scaled from 100% to 400% display scale.

| Where | 100% | 125% | 150% | 200% | 250% | 300% | 400% |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Title bar, context menu, notification area | 16 | 20 | 24 | 32 | 40 | 48 | 64 |
| Taskbar, search, Start all apps | 24 | 30 | 36 | 48 | 60 | 72 | 96 |
| Start pins | 32 | 40 | 48 | 64 | 80 | 96 | 256 |

Use `open-touchdeck.ico` for the executable's icon resource, the window icon and the installer. Use `tray.ico` for the notification area. The 256 px entry is stored as PNG and the others as 32-bit bitmaps with alpha, the layout every Windows icon loader and resource compiler reads.

## How each size is drawn

Every size is its own drawing on its own pixel grid. No entry is a scaled copy of another. Each SVG starts with a comment that gives its grid.

| Sizes | Drawing |
| --- | --- |
| 16, 20, 24 | Flat colours. The body is as wide as the icon, and every Tile edge is on a whole pixel. No pager |
| 30, 32, 36, 40, 48 | Hand-fitted too. Adds shaded glass and Tiles, the screen and the pager, and from 32 px a one-pixel glow around the lit Tile. From 36 px the body has clear pixels each side |
| 60, 64, 72, 80, 96 | The master redrawn on each grid, so the body, the screen, the Tiles and the pager all start and end on whole pixels. Has the soft glow and the contact shadow |
| 256 | The master |

The tray set uses the 16, 20 and 24 px app drawings. Its 32 px drawing is the 16 px one at twice the size, so the tray icon looks the same at every display scale. The 32 px app drawing differs because it is also a Start pin, where the pager fits.

**The rim.** The glass is close to the colour of a dark taskbar (`#202020`), so up to 96 px the body has a one-pixel rim that is lighter than the taskbar. It keeps the outline of the Device on a dark taskbar. On a light one (`#f3f3f3`) it reads as the edge of the case, not as an outline. At 16 and 20 px the Tiles touch the rim, so there it is also darker than the Tiles. From 30 px it is brightest at the top, and the case edge closes the bottom.

## Colours

| Part | 16 to 24 px and tray | 30 to 48 px | 60 to 256 px |
| --- | --- | --- | --- |
| Rim | `#4b5564` at 16 and 20 px, `#566072` at 24 px and in `tray-32` | `#7d889b` at the top, `#4e5766`, then `#454d5a` | `#868d98` at the top, `#3f444b`, then `#2a2d31`. At 256 px, white at 42%, 12% and 6% |
| Glass | `#151a21` | `#272c34` to `#15171a` | `#30363f` to `#15171a` |
| Screen | none | `#0c1826` to `#081018` | `#0c1826` to `#081018` |
| Case edge | `#66738a` | `#5f6b80` | `#6b7789` to `#3a4350` |
| Unlit Tiles | `#4f77b4` | from `#4a6ea6` to `#3f5f93` at 30 px, darker with each size, to `#35507d` to `#283f68` at 48 px | `#2e4263` to `#20304a` |
| Lit Tile | `#8ccaff` | `#a4d6ff` to `#4c9df6` | `#a4d6ff` to `#4c9df6`, with a white edge at 35% |
| Glow and pager pill | none | `#58a8ff` | `#58a8ff` |
| Pager dot | none | `#606470` | `#606470` |

`#58a8ff` is the Device's backlight blue and `#15171a` its black glass, the same colours the app uses when it draws a Device. Unlit Tiles are brighter at small sizes so that the grid still reads.

## Rebuild

`build.py` needs Python 3, [Pillow](https://pypi.org/project/pillow/) and `rsvg-convert` from librsvg (`librsvg2-bin` on Debian and Ubuntu, `librsvg` in Homebrew and MSYS2).

```sh
python3 -I app/icons/app/build.py
```

It renders each SVG at the size the SVG declares, writes `png/` and both `.ico` files, then reads the `.ico` files back and prints each entry's size and format and whether it has the same pixels as its PNG. It stops if a drawing is asked for at a size other than its own. Pass `--rsvg-convert <path>` when `rsvg-convert` is not on `PATH`.

The committed files were built with rsvg-convert 2.61.3 and Pillow 12.1.1. Building twice gives the same bytes. Another librsvg version may differ in a few anti-aliased corner pixels.

To check the committed `.ico` files without rendering (Pillow only), and to draw a sheet of every entry at actual size and enlarged, on `#f3f3f3` and `#202020`:

```sh
python3 -I app/icons/app/build.py --check --sheet .verify/icon-final/check-sheet.png
```

Look at the sheet after any change to a drawing: 48 px and smaller must keep straight edges on whole pixels, and the tray icon must read on both taskbar colours.
