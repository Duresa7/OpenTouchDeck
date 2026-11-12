# Design system

This is the look of Open TouchDeck: the desktop app and the screens the Firmware draws on a Device. Every UI ticket follows it. The tokens live in [`app/ui/src/styles/tokens.css`](../app/ui/src/styles/tokens.css), and this page explains each one, then shows how the parts are built from them.

Words follow [`GLOSSARY.md`](../GLOSSARY.md). Every user-visible string comes from [`docs/copy.md`](copy.md). Strings quoted on this page describe a state; where one differs from the copy guide, the copy guide wins.

## The direction: Backlit

**The app is the desk, and each Device is a lit object sitting on it. Only lit things have color.**

A Device is a 5-inch, 1024 × 600 touchscreen with a black glass bezel. The app draws it as that object, on a quiet desk mat, so the Device is always the brightest and most colorful thing in the window. The chrome around it is the unlit case: solid, small and grey. Color appears only where something glows: the Device's screen, the status lights and your focus.

### Principles

1. **Only lit things have color.** The screen, the status lights and the focus ring glow. Everything else is ink on a surface. Primary buttons are ink, not blue.
2. **The Device is an object, not a card.** It has a bezel and a contact shadow. No light spills onto the mat, and no rounded panel wraps it.
3. **Chrome is small, solid and quiet.** Navigation, the inspector and dialogs never compete with the screen. Surfaces are opaque. No blur and no translucent panels: blur over a busy dark screen hurts legibility and costs GPU time in Electron.
4. **The app borrows the Device's vocabulary.** The current Page wears the pager pill's accent, Preset tiles are drawn in screen colors, and the app and the Firmware share one typeface, Archivo.
5. **Each color has one meaning.** Green means connected. Amber means something waits for you. Blue is the Device's accent and your keyboard focus. Selection in the app's own chrome uses ink, so blue stays unambiguous.
6. **Status is a light plus words.** Never color alone.
7. **Disabled looks different from quiet.** A secondary action and a meaningful empty value use secondary text. Disabled controls lose their edge and fade further.

## Tokens

Components use semantic tokens only. The `--palette-*` values exist so the semantic tokens can derive from them; don't use them in components. Never write a raw color in a component.

Light is the default. Dark follows `prefers-color-scheme`. The Appearance setting forces a theme by setting `data-theme="light"` or `data-theme="dark"` on `<html>`. The two dark blocks in `tokens.css` must stay identical; the contrast script checks this.

### Palette

| Token | Value | Role |
| --- | --- | --- |
| `--palette-ash-felt` | `#e4e6e9` | Light desk mat |
| `--palette-night` | `#000000` | Dark desk mat: true black. The bezel's edge highlight and shadow set the glass apart from it |
| `--palette-bezel` | `#15171a` | The Device's black glass, both themes |
| `--palette-screen` | `#081018` | The Device's screen background |
| `--palette-backlight` | `#58a8ff` | The Device's accent, selection on the Device, dark-theme focus |
| `--palette-live` | `#40d890` | Connected |
| `--palette-signal` | `#ffb020` | Something waits for you: updates, Needs Firmware |

### App colors

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `--mat` | `#e4e6e9` | `#000000` | Window background |
| `--mat-grain` | 0.045 | 0 | Opacity of the felt grain over the mat; none on black |
| `--surface` | `#f6f7f8` | `#262d35` | Nav rail, inspector rail, Settings categories, Integrations list |
| `--surface-raised` | `#ffffff` | `#2c343d` | Dialogs, popovers, the Update icon, inputs, secondary buttons |
| `--surface-inset` | `#eceef0` | `#1b2026` | Grouped Settings rows, change list, tracks, the Layout editor's Pages column |
| `--ink` | `#1a1d22` | `#eef1f5` | Primary text and icons |
| `--ink-2` | `#565d68` | `#a3acb8` | Secondary text, enabled secondary actions such as **Add Page**, meaningful empty values such as "Nothing" |
| `--ink-disabled` | `#a3a9b1` | `#5f6873` | Disabled text only |
| `--line` | ink at 11% | ink at 9% | Hairlines, rings around raised surfaces |
| `--line-strong` | ink at 20% | ink at 17% | Secondary button border |
| `--line-control` | `#7f858e` | `#7d8591` | Edges that identify a control: inputs, selects, switch off, slider thumb ring |
| `--fill-hover` | ink at 5.5% | ink at 6% | Hover, segmented track |
| `--fill-active` | ink at 9% | ink at 10% | Current nav button, open trigger, Add Tile while open |
| `--fill-disabled` | ink at 4% | ink at 4% | Disabled button fill |
| `--accent` | `#1f6fd1` | `#58a8ff` | Current Page's ring in the Pages column, progress, slider fill |
| `--focus-ring` | `#2f7bd9` | `#58a8ff` | The one focus ring |
| `--live-led` / `--live-ink` | `#2fbf71` / `#17804a` | `#40d890` / `#40d890` | Connected light / switch on |
| `--led-off` | `#8a919b` | `#6e7884` | Not connected light |
| `--signal-led` / `--signal-ink` / `--signal-tint` | `#f5a000` / `#8a5600` / `#fdedc8` | `#ffb020` / `#ffc65a` / amber at 14% | Update dot, Firmware update badge |
| `--danger-ink` | `#b42a32` | `#ff7b80` | Clear Tile |
| `--button-primary-bg` / `-ink` | `#1a1d22` / `#ffffff` | `#eef1f5` / `#15191e` | Primary button, tooltip |
| `--scrim` | `rgba(20, 24, 30, 0.28)` | `rgba(8, 10, 14, 0.55)` | Behind dialogs |

The light `--focus-ring` is a step lighter than `--accent` on purpose. It's the one blue that clears 3:1 both on the light mat and on the dark screen, so the same ring works on a Tile.

### Screen colors

The Device's screen is a backlit LCD, so these are the same in both themes. The Firmware uses the same values. Every one is RGB565-exact; the table with each RGB565 value is under [Device translation](#device-translation).

| Token | Value | Use |
| --- | --- | --- |
| `--screen-bg` | `#081018` | Screen background |
| `--screen-tile` | `#182030` | A Tile |
| `--screen-tile-raised` | `#202c40` | A pressed Tile on the Device, the selected Tile in the editor |
| `--screen-track` | `#303848` | The empty part of bars and progress |
| `--screen-pager-idle` | `#606470` | Pager dots for the other Pages |
| `--screen-outline` | `#282c30` | Ring around a Tile that needs newer Firmware |
| `--screen-ink` | `#f0f0f8` | Labels and values |
| `--screen-ink-2` | `#9098b0` | Page title, Tile labels under an icon, second lines |
| `--screen-accent` | `#58a8ff` | Selection, current pager pill, default graph and bar color |
| `--screen-live` | `#40d890` | Running light |
| `--screen-signal` | `#ffb020` | Stopped light, "Needs Firmware" |
| `--screen-danger` | `#ff5860` | Errors on the Device |

**Tile colors** are the backgrounds a user can pick for an action Tile. Labels on them use `--screen-ink`.

| Token | Value |
| --- | --- |
| `--tile-color-violet` | `#483cc0` |
| `--tile-color-green` | `#107050` |
| `--tile-color-blue` | `#205cb8` |
| `--tile-color-red` | `#b02830` |
| `--tile-color-orange` | `#a05810` |
| `--tile-color-slate` | `#606470` |

**Live tile accents** color a graph line and a bar fill. The default is the backlight.

| Token | Value |
| --- | --- |
| `--tile-accent-backlight` | `#58a8ff` |
| `--tile-accent-violet` | `#a08cff` |
| `--tile-accent-coral` | `#ff8868` |
| `--tile-accent-mint` | `#60d4c0` |

**Offline preview.** `--screen-offline-opacity` (0.7) and `--screen-offline-grayscale` (1) draw a Device that isn't connected: its saved Page at 70% over `--screen-bg`, with all color removed. Unlit things have no color, but the Layout stays readable. This is how the app pictures an unplugged Device; the Firmware never uses it.

### Type

One family, Archivo, at two widths. `--font-ui` is `'Archivo Variable', 'Archivo', 'Bahnschrift', 'Segoe UI', system-ui, sans-serif`.

- **Text** (`--stretch-text`, 100%): everything by default.
- **Nameplate** (`--stretch-plate`, 118%, weight 620): Device names, page titles and dialog titles. Wide, flat-sided lettering, like a badge on hardware.

| Size token | Size / leading | Tracking | Use |
| --- | --- | --- | --- |
| `--text-12` | 12 / 16 | +0.01em | Field labels, captions, badges, Settings row hints |
| `--text-14` | 14 / 20 | 0 | Body, buttons, status |
| `--text-16` | 16 / 22 | −0.004em | Inspector title, Integration name, empty-state body |
| `--text-18` | 18 / 24 | −0.008em | Device names, dialog titles (nameplate) |
| `--text-24` | 24 / 28 | −0.014em | Page titles, one Device's name on Home, empty-state title (nameplate) |
| `--text-36` | 36 / 40 | −0.022em | Reserved for onboarding |

Each size has `--leading-*` and `--track-*` tokens. Weights are `--weight-regular` 420, `--weight-medium` 520 and `--weight-semibold` 620. Numbers that update (volume, progress, megabytes) use `font-variant-numeric: tabular-nums`.

### Spacing, radius and elevation

Spacing is a 4 px grid: `--space-1` 4, `-2` 8, `-3` 12, `-4` 16, `-5` 20, `-6` 24, `-8` 32, `-10` 40, `-12` 48, `-14` 56, `-16` 64.

Radius follows physical hierarchy: the bigger and more physical a thing is, the rounder.

| Token | Value | Use |
| --- | --- | --- |
| `--radius-chip` | 999px | Badges, progress, pager |
| `--radius-control` | 8px | Buttons, inputs, nav buttons |
| `--radius-group` | 12px | Update icon, grouped rows, popovers |
| `--radius-sheet` | 16px | Dialogs, the Integrations list |
| `--radius-tile` | 18px | Tiles, in screen pixels at 1024 × 600 |

The Device's case is `--bezel` with a 1px `--bezel-edge` highlight (white at 7%). It scales with the width it's drawn at. Multiply each ratio by that width in px: `--bezel-pad` 0.034 is the bezel thickness, `--bezel-radius` 0.049 the case corner and `--bezel-screen-radius` 0.012 the glass corner.

| Token | Use |
| --- | --- |
| `--shadow-device` | A Device on the mat: a 1px contact line, a tight shadow and a wide ambient one |
| `--shadow-device-lifted` | A Device on Home under the pointer: 2px higher, longer shadow |
| `--shadow-float` | Toasts and error cards, menus, the editor's banner, switch and slider thumbs |
| `--shadow-float-filter` | The Update icon. It's clipped, which would cut a box-shadow, so it casts this drop-shadow from its wrapper |
| `--shadow-dialog` | Dialogs and popovers, always with a 1px `--line` ring |

### Focus

`--focus-ring-width` (2px), `--focus-ring-offset` (2px) and `--focus-ring` make the one focus ring. Every focusable element gets it from a single `:focus-visible` rule:

```css
:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
```

A few places change where the ring is drawn. All of them keep the token's color and width:

- **Swatches** push the ring 4px further out, so it sits outside the chosen ring instead of on top of it.
- **The slider** draws the ring on its thumb, because the focused element is the thumb's hidden input.
- **The editor canvas** is scaled, so its rings divide by the canvas scale to stay 2 app pixels wide. On a Tile the ring sits inside the Tile edge. The selection ring sits outside, so a selected Tile with focus shows both.

## Window frame

The Electron window draws the desk mat to the top edge on Windows and Linux,
using a hidden title bar and Window Controls Overlay. The system owns the caption
buttons and their hover behaviour (including Windows Snap layouts). The window
keeps the title **Open TouchDeck** for the taskbar and assistive technology.

A thin drag strip occupies the reported `titlebar-area-*` rectangle. Drag,
double-click and the system menu use Electron's native drag region; interactive
controls inside it are `no-drag`. All screens and modal content reserve the
reported top inset, including after resize, maximize, fullscreen or scale changes.
Browser tabs reserve none. The hidden app menu keeps its keyboard shortcuts.
Caption backgrounds and glyphs follow Appearance immediately; Match Windows
also follows system theme changes. No Mica or acrylic is used.

## Components

Each part below is in the reference prototype; [Reference prototype](#reference-prototype) lists the screenshot for each state.

### Nav rail

The rail isn't in the reference prototype. It's variant C of the Layout editor prototype on the `prototype/layout-editor-variants` branch.

- Docked to the window's left edge for its full height, below the drag strip in the Electron window (see [Window frame](#window-frame)): 64 wide, `--surface`, with a 1px `--line` right edge. Every screen lays out to its right, from 800 × 600 up. Nothing on a screen moves it: not switching screens, not scrolling, not the Pages column or the Details panel opening or closing.
- Home, Layout, Integrations and Settings sit at the top, 12px down with 2px gaps, centred in the rail. The Update icon sits at the bottom, 12px up.
- Buttons are 36 × 36, radius 8, with a 20px Lucide icon at stroke 1.9 in `--ink-2`. Hover: `--fill-hover` and `--ink`. Current: `--fill-active` and `--ink`, with `aria-current="page"`. Settings is current while its page is open.
- A tooltip to the right of each button names it after a 500ms delay. Later tooltips open instantly while the pointer stays in the rail.
- It comes first in the tab order, before the screen's content.
- Toasts and error cards centre on the content to the right of the rail, 24px up from the bottom.
- Onboarding hides it but keeps its column, so the screen behind doesn't shift.

### Update icon

At the bottom of the nav rail, centred in it. Open, its label runs out to the right over the screen, so nothing it does moves the nav icons. Only the visible part of the icon takes clicks. It isn't rendered when no update exists.

| State | Icon | Label |
| --- | --- | --- |
| Available | Refresh icon with an 8px `--signal-led` dot | Update available |
| Downloading | 22px progress ring: track `--line-strong`, fill `--accent`, stroke 2.5. No dot | Downloading… {pct}% |
| Ready | Refresh icon with the dot | Ready to restart |

- Closed, it shows a 44 × 44 icon. It opens to show the label on mouse hover and on keyboard focus, and closes on leave and blur. Touch never opens it; a tap goes straight to the dialog.
- It's always laid out at full width and `clip-path: inset()` reveals it, so opening it costs no layout. The clipped part takes no pointer hits. Open, the clip reaches past the edge far enough to show the focus ring.
- The button's `aria-label` is the label. The visible label is `aria-hidden`.
- Clicking it opens the Update dialog at the current step.

### Update dialog

440 wide, 24px padding, `--radius-sheet`, `--surface-raised`, a `--line` ring and `--shadow-dialog` over the scrim. The title is nameplate 18/620.

1. **Offer.** The version as the title, the current version under it, then a "What's new" heading over a change list in a `--surface-inset` group. Buttons: Later (secondary) and Update (primary). Nothing downloads before Update.
2. **Downloading.** A 6px progress bar (`--surface-inset` track with a `--line` ring, `--accent` fill drawn with `scaleX`) and a 12px tabular row with megabytes and percent. Hide closes the dialog; the download continues and the Update icon shows its ring.
3. **Ready.** Titled **Ready to restart**. The body names the version and says what restarting does. Buttons: Later and Restart (primary).

Opened from the keyboard, focus lands on the first control. Opened with the pointer, focus goes to the dialog itself, so no ring appears. Escape closes it and focus returns to the Update icon. Each dialog has at most one primary button.

### Settings

A full screen, like Home or Integrations, reached from the rail's Settings icon. Switching to it never animates.

- **Categories** in a 232-wide `--surface` column right of the nav rail, with a 1px `--line` right edge, sticky to the window's height: the nameplate title "Settings", then **General**; a **Devices** label over one entry per Device, named by it; an **App** label over **Paired browsers**, **Troubleshooting**, **Errors** and **About**. Labels are 12/620 `--ink-2`. An entry is 36 tall with a 16px icon and its name in 14/520 `--ink-2`; hover is `--fill-hover`, the current one `--fill-active` in `--ink` with `aria-current="page"`. **Errors** shows its count in a `--danger-ink` pill. Below 760 wide the column sits above the content.
- **Content,** at most 760 wide with 40 × 48 padding: a 24/620 nameplate page title, which takes focus when the category changes, and an optional `--ink-2` lede. Sections have a 14/620 heading over a group of rows in `--surface-inset` with a `--line` ring. A row is at least 56 tall: title 14/520 and an optional 12px hint in `--ink-2` on the left, the control on the right. Rows inside a group are split by a `--line` hairline. Controls in rows: a switch, a segmented control (the theme: System, Light, Dark) or a secondary button.
- **A Device's page** is titled with its name, its status as the lede. Its fields sit in `--surface-inset` cards with 16px padding and 20px between fields, under the headings General (Name, Orientation), Screen (Brightness, with the value tabular at the label's right; Photo frame, a switch, then Start after and Folder when on), Wireless (Wi-Fi, Bluetooth keyboard) and Firmware (the version and its status with Update and Restart, then the Firmware build). A Device that isn't connected leads with a `--signal-tint` line asking to plug it in.
- **Errors** lists what went wrong since the window opened, newest first, at most 50: a `--surface-inset` card each with the message in 14/520, the time and where in 12px `--ink-2`, a Copy icon button, and the detail in a `--surface-raised` monospace block. **Clear** empties it.

### Home

Home has no headline. The Devices are the content, centered on the mat.

- **One Device:** drawn 560px wide, name at 24 nameplate.
- **Two or three:** 312px each, in one row with 56px gaps.
- **More than three:** the row wraps. Not prototyped yet.
- **Caption under each Device**, aligned to its left edge: the name in nameplate 18/620, cut with an ellipsis. There is no status light: the lit screen says the Device is connected, and screen readers hear the name with its status. A Device that isn't connected shows its status words and when it was last seen. A Firmware update adds the amber badge.
- **The Device is the button.** Clicking it opens its Layout. On a fine pointer it lifts 2px with `--shadow-device-lifted`.
- **Connected:** the lit screen.
- **Not connected:** the saved Page stays visible through the offline preview tokens. The status and last-seen words show under the name.
- **Controls need Send to Device:** a connected Device whose controls wait for **Send to Device** keeps its status row and adds a second row under it: a `--signal-led` light with its words, then a small secondary **Review Layout** button. After Send to Device it shows a `--led-off` light with "Checking controls…" until the Companion reports a match, or a `--signal-led` light with "Couldn't check…" when the Companion can't read the Layout. Both are polite live regions, and the light sits on the first line when the words wrap.
- **No Devices:** a switched-off Device silhouette, 360px at 55% opacity, then a nameplate title, a 16px body in `--ink-2` that asks for a USB data cable, a 14px line about New boards, and a secondary **Connection help** button. No promise about how long detection takes. Max width 440.

### Layout editor

A grid of the Pages column against the nav rail, the canvas under its header, and the Details panel on the right edge, 340 to 600 wide, resized from its left edge.

**Pages column.** It isn't in the reference prototype. It's variant C of the Layout editor prototype on the `prototype/layout-editor-variants` branch, like the nav rail. It runs the editor's full height, 160 wide, in `--surface-inset` with a 1px `--line` right edge and 12 × 10 padding, and scrolls on its own. It lists the Pages like a presentation app's slides, 8px apart:

- **An entry** is a button, radius 8, padded 6 × 6 × 6 × 2: the Page number on the left in 12px tabular `--ink-2`, then a thumbnail, then the Page name under it on one line in 12/520, cut with an ellipsis and whole in its `title`. The name is `--ink-2`, and `--ink` for the current Page. A Usage page keeps its **Usage** tag after the name. Hover is `--fill-hover`. Clicking an entry shows that Page.
- **The thumbnail** is the Page as the Device draws it, from the same scene as the canvas: its name, Tiles or Usage rows, and pager, in screen colors, at the Device's shape with a 112px long side, radius 5. Landscape and portrait Layouts each get their own shape. It follows every edit as you make it.
- **The current Page** has `aria-current` and a 2px `--accent` ring around its thumbnail with a 2px gap of `--surface-inset`, the pager pill's color. The column scrolls to keep it in view however the Page changes, and keyboard focus on another entry moves to it.
- **An entry's menu** opens from a right-click (a long press on touch) at the pointer, or from a 24px **…** button over the thumbnail's top right corner: `--surface-raised` with a `--line` ring, radius 6. The button shows on hover and keyboard focus, while its menu is open, and always on a touch screen. Its tooltip is **More**. The menu holds **Remove Page** in `--danger-ink` with a trash icon. The first click arms it, as the Details panel's trash can did: it reads “Remove “{page}”?” over "Click again to remove", and the second click removes the Page. It disarms after 4 s or when the menu closes. With one Page left it's disabled and says why. After a removal, focus goes to the current Page's entry.
- **Add Page** ends the list: 40 tall, a 1.5px dashed `--line-strong` edge, radius 8, a plus and its words in 12/520 `--ink-2`; hover `--fill-hover` and `--ink`. At 8 Pages it's disabled: `--ink-disabled` on a `--line` dashed edge, with the tooltip "A Layout holds up to 8 Pages".

**Folding the Pages column.** The button at the left end of the header folds the column away and brings it back. The column's width animates over `--duration-expand` with `--ease-drawer`; the column keeps its width as it slides out to the left behind the rail, then turns invisible. It's inert while folded, so it leaves the tab order and screen readers. The canvas and the Device grow into the room. This PC remembers the choice across restarts; when storage isn't available, the column stays out and folding lasts until the window closes.

**Narrow windows.** When the editor is narrower than the Pages column, the smallest canvas and the smallest Details panel side by side (160 + 460 + 340), only one panel is out at a time. The Details panel wins while it's out, so picking a Tile or Add Tile folds the column aside; the Pages button brings the column back and hides the Details panel. The column keeps its remembered state underneath.

**Hiding the Details panel.** The panel button at the right end of the header hides it. The panel slides out to the right over `--duration-expand` with `--ease-drawer`, keeping its width so nothing inside squeezes, and the canvas takes its room. Selecting a Tile (a click or Enter, not the press that starts a drag), **Add Tile**, an empty cell, **Review** in the controls notice, or an approval waiting for this Device brings it back. Hiding it ends Add Tile.

**Header,** 64 tall over the canvas: the Pages button on the left, then the Device's name in nameplate 18/620 with a 32px pencil icon button in `--ink-2` after it, then on the right the orientation buttons, Preview, the Layout menu, **Add Tile** (a secondary button with a plus icon), **Send to Device** and the Details panel button. There is no save state and no connection light. The pencil's tooltip and Send to Device say when the Device isn't connected, and screen readers hear the connection with the Device's name.

**Renaming.** The pencil swaps the name for a 240-wide input at the same size, its text selected. Enter or leaving the field saves the name to the Device; Escape keeps the old one. Enter and Escape put focus back on the pencil. A name the Device refuses keeps the field open with the reason in a small raised card under it, in `--danger-ink`.

**Send to Device** shows only while the Layout has changes the Device lacks, or while its controls need a send. A click turns it into a spinner and "Sending…", then a check and "Saved" for 1.6 s, then it folds to nothing. Its width follows its words, measured, and slides over `--duration-expand` with `--ease-drawer`; it folds over `--duration-collapse`, and each new look fades in over `--duration-label`. The spinner turns once per `--duration-spin`. The Layout menu's **Send to {device}** sends again without a change.

**Canvas.** The Device, centered, drawn between 420 and 680 wide to fit, or up to 860 with the Details panel hidden, with no light spill.

On the canvas, the editor adds marks the Device never draws, in screen pixels:

- **Selected Tile:** `--screen-tile-raised` with a 5px `--screen-accent` ring 6px outside it.
- **Empty cell:** a 3px dashed `--screen-pager-idle` outline at `--radius-tile` with a 44px plus in `--screen-ink-2`. Clicking one opens Add Tile for that cell.
- **Add Tile target:** the cell gets a 5px solid `--screen-accent` outline, a `--screen-tile` fill and an accent plus.

**Controls notice.** Above the Device on the canvas, at most 680 wide, while the open Device's controls wait for **Send to Device**: `--signal-tint` with a `--line` ring and `--radius-group`, a 14/620 `--signal-ink` title led by a `--signal-led` light, one 12px paragraph in `--ink` whose last sentence is `--ink-2`, and one small secondary button on the right. The canvas counts the notice's full height when it fits the Device. Below 700 tall, the canvas over the Device pads 16 on top and spaces its parts 16 apart while any notice shows, and the notice pads 8 × 12. When the Device is already at its 420 minimum, the notice gives up height before anything else and scrolls inside itself, never below its title and one line, so the status line stays in the window. While Checking it's one `--surface-raised` line with a `--led-off` light. When the check can't read the Device it's the same line in `--ink` with a `--signal-led` light. The inspector's Page details then start with a **Controls** list in a `--surface-inset` group: each row names the Tile, its Page, the Integration and account, and its controls, and selects the Tile. Unknown names use `--danger-ink`.

**Status line.** Under the Device, 32px down, 12px `--ink-2`. It reports refusals, imports and sends. A load from the Device shows nothing there, but screen readers still hear it.

**Error card.** An error pops up as a toast card at the bottom centre of the content right of the nav rail instead of a line: the error icon in `--danger-ink`, the message, where it happened in 12px `--ink-2`, and a small **Details** button that opens **Settings > Errors**. It stays 10 s, longer while the pointer rests on it. The status line keeps the words for screen readers only.

**Page details.** The Page name, then **Grid**: an 8 × 8 board of 4px-gapped squares, 220 wide, with the size in words at the label's right. Squares up to the current size are `--accent`; hovering or focusing a square previews its size at 40% `--accent` and names it, and a click, Enter or Space sets it. Only one square is in the tab order, and the arrow keys, Home and End move between them. Removing a Page is in its menu in the Pages column.

**Inspector.** `--surface` with a 1px `--line` left edge, 20px padding and 20px between fields. The head has a 44px Tile thumbnail in screen colors, the Tile's name at 16/620 and a 12px line saying what it is and where. The footer stays pinned to the bottom while the fields scroll: Duplicate on the left, Clear Tile (quiet danger) on the right.

| Action Tile | Live tile |
| --- | --- |
| Label | Label |
| Icon: the icon, its name, Change | Integration, a select led by the Integration's icon |
| Background: Default, the six Tile colors and a custom color picker | Value |
| Size: 1 × 1, 2 × 1, 2 × 2 | Display: Graph and value, or Value only (graph Tiles) |
| When tapped | History (graph Tiles) |
| When held. "Nothing" is a real choice and uses `--ink-2` | Accent: the four Live tile accents |
| | Size. Sizes the Tile can't draw are disabled, such as 1 × 1 for a graph |

**Add Tile** replaces the inspector while it's open, so the Page and the target cell stay in view. A popover from the header button would cover the canvas.

- The head says where the Tile goes: the row, column and Page. On a full Page it says the Tile starts a new Page.
- **Action:** the action kinds in a two-column grid. Each is a bordered button with a 32px icon tile in screen colors, a 12/620 name and a 12px `--ink-2` line.
- **Preset tiles:** grouped under each connected Integration, drawn as Preset tile previews.
- Cancel sits in the pinned footer, and Escape also cancels. Picking a choice places the Tile and selects it, so the inspector shows its fields.
- While it's open, the Add Tile button shows `--fill-active` and `aria-expanded="true"`.

### Integrations

A page title in nameplate 24 with a 14px `--ink-2` line under it, then one list sheet: `--surface`, `--radius-sheet` and a `--line` ring. It's a list, not a card grid.

Each row is a grid of a 40px glyph, the name (16/620) with one line about it in `--ink-2`, the Integration's Preset tiles drawn as 84 × 56 previews in screen colors, and its state on the right. The state is a status light with words, or a **Set up** or **Create** button. The Custom Integration's line says what the user gets, not how it works; URLs, JSON and commands belong to its setup flow.

### Buttons

| Kind | Look |
| --- | --- |
| Secondary, the default | 34 tall, 14px padding, `--surface-raised`, a 1px `--line-strong` border, `--ink` at 14/520. Hover mixes in 5% ink |
| Primary | `--button-primary-bg` and `--button-primary-ink`: ink, not blue. One per dialog |
| Quiet danger | No border or fill, `--danger-ink` |
| Small | 28 tall, 10px padding, 12px text |
| Icon | 32 × 32, no border or fill, `--ink-2`; hover `--fill-hover` and `--ink` |
| Open toggle | `--fill-active` while `aria-expanded="true"` |
| Disabled | No border, `--fill-disabled`, `--ink-disabled`, `cursor: not-allowed`, no press |

Pressing a button scales it to `--press-scale`.

### Fields

- **Label:** 12/620 `--ink-2`, 8px above the control.
- **Input and select:** 36 tall, radius 8, a 1px `--line-control` edge on `--surface-raised`. A select is a button showing the value and an up-down chevron in `--ink-2`, with an optional leading icon.
- **Segmented control:** a `--fill-hover` track with a `--line` ring and 2px padding. Segments are 28 tall at 12/520 in `--ink-2`. The chosen one is `--surface-raised` with `--ink` and a hairline ring. A disabled segment uses `--ink-disabled`.
- **Switch:** 40 × 24. Off is `--line-control`, on is `--live-ink`, and the 20px white thumb slides across.
- **Slider:** a 6px `--surface-inset` track with a `--line` ring, an `--accent` fill and an 18px white thumb ringed in `--line-control`.
- **Swatches:** 28px circles. The chosen one shows a check, a 2px `--surface` gap and a 2px `--ink` ring. The check is white on Tile colors and `--screen-bg` on the light accents.

### Selection and focus

| | In the app | On the Device and canvas |
| --- | --- | --- |
| Current or chosen | `--fill-active`, a raised segment, or an `--ink` ring | `--screen-accent` ring, current pager pill |
| Keyboard focus | The focus ring | The focus ring, inside the Tile |
| Hover | `--fill-hover` | Empty cells brighten to `--screen-ink-2` |

Popovers and dialogs follow one rule. Opened from the keyboard, focus lands on the first control. Opened with the pointer, focus goes to the popup itself. Closing returns focus to the trigger.

### Status lights, badges and tooltips

- **Status light:** an 8px dot before its words. Connected is `--live-led` with a 3px halo of the same color at 18%. Not connected is `--led-off`.
- **Badge:** a 24 tall chip in `--signal-tint` with `--signal-ink` text at 12/620 and a 14px icon. Used for a Firmware update.
- **Tooltip:** `--button-primary-bg` with `--button-primary-ink`, 12/520, radius 7, 5 × 9 padding, 8px from its trigger. A tooltip never repeats a visible label.

## Motion

Motion gives feedback and shows where things come from. It never decorates. Animate only `transform`, `opacity` and `clip-path`, plus color on hover. Never animate `width`, `height`, `top`, `left`, margins or blur.

| What | Property | Duration | Easing |
| --- | --- | --- | --- |
| Press on buttons, nav buttons, Add Tile choices | `scale` to `--press-scale` (0.97) | `--duration-press` 120ms | `--ease-out` |
| Press on a Device on Home | `scale` to `--press-scale-device` (0.985) | 120ms | `--ease-out` |
| Hover color | background, color | `--duration-hover` 150ms | `--ease-color` (`ease`) |
| Device lift on Home (fine pointer only) | `translateY(--lift-device)` (-2px), and the lifted shadow crossfades over the resting one by `opacity` | `--duration-lift` 150ms | `--ease-out` |
| Tooltip in | opacity, scale `--enter-scale-tooltip` (0.96) to 1 from its trigger | `--duration-tooltip` 140ms | `--ease-out` |
| Popover in | opacity, scale `--enter-scale` (0.97) to 1 from the trigger | `--duration-popover` 180ms | `--ease-out` |
| Dialog in | opacity, scale `--enter-scale` (0.97) to 1 from the center; the scrim fades | `--duration-dialog` 220ms | `--ease-out` |
| Any exit | the reverse of its enter | `--duration-exit` 140ms | `--ease-out` |
| Update icon opens | `clip-path` | `--duration-expand` 280ms | `--ease-out` |
| Update icon closes | `clip-path` | `--duration-collapse` 200ms | `--ease-out` |
| Update label in | opacity, after `--delay-label` 60ms | `--duration-label` 160ms | `--ease-out` |
| Update label out | opacity | `--duration-label-out` 140ms | `--ease-out` |
| Switch thumb | `translate` | `--duration-switch` 180ms | `--ease-out` |
| The Pages column or the Details panel folds and unfolds | the editor's grid columns; the panel keeps its width and slides | `--duration-expand` 280ms | `--ease-drawer` |
| Send to Device grows to its words | `width`, measured | `--duration-expand` 280ms | `--ease-drawer` |
| Send to Device folds away after Saved | `width`, opacity | `--duration-collapse` 200ms; fades in `--duration-label-out` | `--ease-drawer` |
| Send to Device's spinner | `rotate`, one turn | `--duration-spin` 800ms, repeating | `linear` |
| A Device's screen lights up as it connects, on Home and in setup | opacity of the screen from `--screen-offline-opacity` | `--duration-screen-light` 240ms; dims in `--duration-exit` | `--ease-out` |
| A dropped Tile settles into its cell | `transform` | `--duration-tile-settle` 160ms | `--ease-out` |
| Download progress | `scaleX` | `--duration-progress` 120ms | `--ease-progress` (`linear`) |
| Theme change | background, color | `--duration-theme` 240ms | `--ease-color` |

Every duration, easing, scale and distance is a token in `tokens.css`; a component never writes its own. Only an instant `0ms` is written in place. `app/ui/src/styles/motion.test.ts` checks this.

`--ease-out` is `cubic-bezier(0.23, 1, 0.32, 1)`. `--ease-in-out` (`cubic-bezier(0.77, 0, 0.175, 1)`) is for things that move on screen, and `--ease-drawer` (`cubic-bezier(0.32, 0.72, 0, 1)`) is for sheets, if one is ever added.

**What never animates:** changing Page, changing section from the nav, selecting a Tile, swapping the inspector between action, Live tile and Add Tile, and anything you do with the keyboard. Keyboard focus opens the Update icon on the next frame. The editor's try-it mode is the one exception: a swipe or a Page-jump Tile slides between Pages with the Firmware's own timing (`THEME.anim`), because it shows what the Device does. The arrow keys change Page there at once.

**Interruptible.** Use CSS transitions, not keyframes, for anything a pointer can reverse. A transition turns back from wherever it is: leaving the Update icon halfway through opening closes it from that width.

**Reduced motion.** Under `prefers-reduced-motion: reduce`, the tokens remove movement and keep fades, in one block at the end of `tokens.css`: every press and entry scale becomes 1, popovers and dialogs fade in 120ms and 140ms without scaling, the Device lift is off, and the switch thumb, the chevron, a Tile settling, progress and every Update icon duration and delay become 0ms, so each jumps straight to its end state. Tooltips, the scrim, hover color, the theme change and a screen lighting up still fade. The try-it Page slide changes Page at once.

## Device translation

The Firmware draws its screens with LVGL at 1024 × 600 in RGB565. To match the app, it takes the screen colors, the geometry and the three Archivo instances below. Nothing else on this page reaches the Device.

### Colors in RGB565

Every screen token is RGB565-exact: red and blue are multiples of 8 and green a multiple of 4, or the channel is 255. The `UI_RGB565` macro in `firmware/components/ui/ui_theme.h` keeps every bit, so the panel shows the token itself. Keep it that way when a color changes; the contrast script fails any lossy screen token.

| Token | Hex | RGB565 | Firmware |
| --- | --- | --- | --- |
| `--screen-bg` | `#081018` | `0x0883` | `UI_RGB565(8, 16, 24)` |
| `--screen-tile` | `#182030` | `0x1906` | `UI_RGB565(24, 32, 48)` |
| `--screen-tile-raised` | `#202c40` | `0x2168` | `UI_RGB565(32, 44, 64)` |
| `--screen-track` | `#303848` | `0x31C9` | `UI_RGB565(48, 56, 72)` |
| `--screen-pager-idle` | `#606470` | `0x632E` | `UI_RGB565(96, 100, 112)` |
| `--screen-outline` | `#282c30` | `0x2966` | `UI_RGB565(40, 44, 48)` |
| `--screen-ink` | `#f0f0f8` | `0xF79F` | `UI_RGB565(240, 240, 248)` |
| `--screen-ink-2` | `#9098b0` | `0x94D6` | `UI_RGB565(144, 152, 176)` |
| `--screen-accent` | `#58a8ff` | `0x5D5F` | `UI_RGB565(88, 168, 255)` |
| `--screen-live` | `#40d890` | `0x46D2` | `UI_RGB565(64, 216, 144)` |
| `--screen-signal` | `#ffb020` | `0xFD84` | `UI_RGB565(255, 176, 32)` |
| `--screen-danger` | `#ff5860` | `0xFACC` | `UI_RGB565(255, 88, 96)` |
| `--tile-color-violet` | `#483cc0` | `0x49F8` | `UI_RGB565(72, 60, 192)` |
| `--tile-color-green` | `#107050` | `0x138A` | `UI_RGB565(16, 112, 80)` |
| `--tile-color-blue` | `#205cb8` | `0x22F7` | `UI_RGB565(32, 92, 184)` |
| `--tile-color-red` | `#b02830` | `0xB146` | `UI_RGB565(176, 40, 48)` |
| `--tile-color-orange` | `#a05810` | `0xA2C2` | `UI_RGB565(160, 88, 16)` |
| `--tile-color-slate` | `#606470` | `0x632E` | `UI_RGB565(96, 100, 112)` |
| `--tile-accent-backlight` | `#58a8ff` | `0x5D5F` | `UI_RGB565(88, 168, 255)` |
| `--tile-accent-violet` | `#a08cff` | `0xA47F` | `UI_RGB565(160, 140, 255)` |
| `--tile-accent-coral` | `#ff8868` | `0xFC4D` | `UI_RGB565(255, 136, 104)` |
| `--tile-accent-mint` | `#60d4c0` | `0x66B8` | `UI_RGB565(96, 212, 192)` |

Two screen colors are mixes rather than tokens, because they follow the user's accent:

- **Graph area:** the accent mixed 14% into `--screen-tile`, which is `lv_color_mix(accent, tile, 36)`.
- **Toggled-on Tile:** a 4px inset ring of `--screen-ink`.

### Geometry

All values are screen pixels at 1024 × 600, landscape.

| Token | Value | What |
| --- | --- | --- |
| `--screen-pad-x` | 28 | Left and right margin of the Tile grid |
| `--screen-pad-top` | 76 | Top of the Tile grid, under the Page title |
| `--screen-pad-bottom` | 56 | Bottom margin, over the pager |
| `--screen-gap` | 16 | Between Tiles |
| `--radius-tile` | 18 | Tile corners |
| `--screen-title-x`, `--screen-title-y` | 36, 22 | Page title position |
| `--screen-pager-dot` | 12 | Pager dot, `--screen-pager-idle` |
| `--screen-pager-current` | 36 | Current Page pill width, `--screen-accent` |
| `--screen-pager-bottom` | 18 | Pager distance from the bottom edge |

Graph lines are 4px in the accent. A second series is 3px in `--screen-ink-2`. Bars are 8px on `--screen-track` with round ends; the volume bar is 28px with radius 10. A Tile that needs newer Firmware has no fill, a 3px `--screen-outline` ring, its label in `--screen-ink-2` and "Needs Firmware {x.y}" in `--screen-signal`.

### Fonts on the Device

The Firmware renders three static instances of Archivo. They're cut from the variable font, which has a width axis from 62 to 125 and a weight axis from 100 to 900:

| Instance | Weight | Width | Sizes (px) | Used for |
| --- | --- | --- | --- | --- |
| Archivo Medium | 520 | 100 | 22, 26, 44 | Second lines, Tile labels, key legends |
| Archivo Semibold | 620 | 100 | 32, 40 | Live tile values and now-playing titles, the one big number on a Live tile |
| Archivo Nameplate | 620 | 118 | 26 | Page title |

The app draws its Device previews with the same values: `--screen-font`, `--screen-weight-medium`, `--screen-weight-semibold`, `--screen-stretch-plate` and the `--screen-text-*` sizes, which are `title` 26, `label` 26, `sub` 22, `value` 32, `big` 40 and `legend` 44.

That makes six LVGL fonts, each at 4 bits per pixel. The glyph set for all but the legend size:

- U+0020 to U+007E, Basic Latin
- U+00A0 to U+00FF, Latin-1 Supplement
- U+2013 and U+2014, the dashes; U+2018, U+2019, U+201C and U+201D, the curly quotes; U+2022, the bullet; U+2026, the ellipsis that in-progress states use; U+2212, the minus sign

The 44px legend font needs Basic Latin only. Icons aren't in the font: Tile icons are Assets the Firmware stores and draws (ADR 0004).

To build an instance, cut it from the variable font with fontTools, then convert it with `lv_font_conv`:

```sh
fonttools varLib.instancer "Archivo[wdth,wght].ttf" wght=520 wdth=100 -o Archivo-Medium.ttf
lv_font_conv --font Archivo-Medium.ttf --size 26 --bpp 4 --format lvgl \
  -r 0x20-0x7E -r 0xA0-0xFF -r 0x2013-0x2014 -r 0x2018-0x2019 -r 0x201C-0x201D \
  -r 0x2022 -r 0x2026 -r 0x2212 -o ui_font_medium_26.c
```

### Desktop only

The Firmware never draws these. They exist only in the app's picture of a Device or in the app's chrome:

- The felt grain on the mat
- The glass reflection across the screen
- The bezel, its edge highlight and every shadow
- Blur of any kind
- The offline preview's dimming and grey
- The editor's marks: selection ring, focus ring, empty-cell outlines and the Add Tile target
- The album-art gradient in the prototype, which stands in for a real cover image

LVGL 9.6 can't dither a gradient, so the Device's own fills stay flat colors.

## Contrast

`contrast.mjs` in the reference prototype measures WCAG 2.2 contrast for every text and UI-part pair in both themes. It runs against `tokens.css`, so a token change shows up on the next run. Text pairs need 4.5:1. Large text and UI parts, including the focus ring, need 3:1.

It also measures:

- text and the focus ring over the felt grain's darkest or lightest pixel
- the offline preview, after dimming and grey
- every Device pair after `UI_RGB565`
- that both dark blocks match
- that every screen token is RGB565-exact

Three kinds of pair are reported but not gated, as WCAG allows: disabled text, status lights (always next to words) and button borders (the label identifies the button).

## Fonts and licenses

- **Archivo** is by the Archivo Project Authors (Omnibus-Type) under the [SIL Open Font License 1.1](https://github.com/Omnibus-Type/Archivo/blob/master/OFL.txt). It declares no Reserved Font Name, so converted instances can keep the name. The app bundles it from `@fontsource-variable/archivo`, whose package license is `OFL-1.1`. Ship the OFL text next to the font files in the app, and with the converted fonts in the Firmware source. Bundling the files is #73's job.
- **Lucide**, the app's interface icons, is ISC-licensed through `lucide-react`.
- Icons for the icon picker follow [`docs/research/icon-sources.md`](research/icon-sources.md).

## Reference prototype

A static React prototype shows every state on this page. It's local only, under the git-ignored `.verify/69/final/`:

| Path | What |
| --- | --- |
| `src/` | The prototype. Every state has a URL hash, such as `#editor-live` |
| `tokens.css` | The same file as `app/ui/src/styles/tokens.css` |
| `contrast.mjs` | The contrast script; `contrast-output.txt` is its last run |
| `motion-check.mjs` | Checks that the Update icon animates by clip only, reverses mid-flight and jumps under reduced motion |
| `shoot.mjs` | Takes every screenshot below and fails if a focus shot has no visible ring |
| `screens/` | The screenshots, light and dark, at 1180 × 760 |

| Screens | Files in `screens/`, each with `-light` and `-dark` |
| --- | --- |
| Home with 0, 1 and 3 Devices, one not connected | `home-empty`, `home-single`, `home-three` |
| Editor, action Tile and Live tile selected | `editor-action`, `editor-live` |
| Add Tile | `editor-add` |
| Device panel | `editor-devicepanel` |
| Every Update state | `update-hidden`, `update-available`, `update-hovered`, `update-dialog`, `update-downloading`, `update-downloadingdock`, `update-ready`, `update-readydock` |
| Keyboard focus | `settings-focus`, `focus-nav`, `focus-update`, `focus-devicename`, `focus-pages`, `focus-inspector`, `focus-dialog` |
| Integrations | `integrations` |

To run it from `.verify/69/final/`:

```sh
npm install
npx vite build && npx vite preview --port 4189 --strictPort
node shoot.mjs http://localhost:4189/
node motion-check.mjs http://localhost:4189/
node contrast.mjs
```
