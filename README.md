# Open TouchDeck

A customizable touchscreen Device for your desk. Design its Layout in the app, then use its Tiles for shortcuts, live readings and controls on this PC.

[**Download for Windows 11**](https://github.com/Duresa7/OpenTouchDeck/releases/latest) · [**User guide**](docs/usage.md) · [**Releases**](https://github.com/Duresa7/OpenTouchDeck/releases)

![Layout editor in light mode, with demo data](docs/images/app-layout-light.png)

I made this project to get around the limitations of numpads and keyboards that lock useful features behind proprietary software. Layouts, actions and Assets stay customizable.

## Features

- Native Windows 11 app in Rust and Slint, with the Companion running in the same process and staying in the tray when you close the window.
- Install the bundled Firmware on a New board over USB, and run guided setup to name a Device and pick Landscape or Portrait.
- Read a Layout from a Device or start from the Starter or blank Layout, add and remove Pages, and add, move, resize and delete Tiles on the Device canvas with the mouse or keyboard before sending over USB.
- Configure This PC, CLIProxyAPI, Custom (HTTP and command sources), Volume and Media Integrations, with secrets kept in Windows Credential Manager or the system keyring and command execution gated by a native approval prompt on this PC.
- Manage each Device from its Settings page: rename it, switch orientation, pick an idle Photo frame folder, manage Wireless paired apps on the radio Firmware build, and update or restart Firmware from the bundled build or a `.tdfw` file with automatic rollback if a new start fails.
- Drive shortcuts, modifiers, media keys, app launches, links, approved commands and Live tiles (CPU, memory, network, CLIProxyAPI usage limits, Custom readings, a draggable Volume bar and Media now-playing controls) from the Device's Layout.

## Get started

You need a **Waveshare ESP32-S3-Touch-LCD-5B** (16 MB flash, 8 MB PSRAM, 1024 × 600 display), a USB data cable and a Windows 11 x64 PC. Windows 10 users stay on the 1.x installer on [Releases](https://github.com/Duresa7/OpenTouchDeck/releases).

If you have Open TouchDeck 1.x installed on Windows 11, quit it and uninstall it from Windows **Installed apps** first (choose **No** when asked whether to delete your Layouts and settings), then install the 2.0 MSI.

1. Download and run `OpenTouchDeck-2.0.0-windows-x64.msi` from the [latest release](https://github.com/Duresa7/OpenTouchDeck/releases/latest). It installs for the current user without administrator rights.
2. Open Open TouchDeck and connect the Device through its USB-Serial/JTAG port. It connects automatically.
3. For a New board, open **Setup**, choose **Install Firmware** and confirm in the native dialog on this PC. This replaces the board's existing project.
4. Follow setup to update Firmware if offered, name the Device and choose **Landscape** or **Portrait**.
5. Open **Layout**, read the Device's Layout or choose **Start a Layout**, arrange Pages and Tiles on the canvas and choose **Send to Device**.

The [user guide](docs/usage.md) covers Home, Setup, the Layout editor, Settings, the Device page and Integrations.

Keep Open TouchDeck running and USB connected for actions on this PC and live readings. Closing the window leaves the app in the tray. It reconnects automatically if the Device drops the connection. Turning an Integration off makes its readings on the Device stale within about a second. Swiping Pages and stored animations work on the Device.

## Device screen examples

These captures show the built-in Layout on a real Device.

<p>
  <img src="docs/images/deck-media.png" width="49%" alt="Media Page with playback and Volume controls">
  <img src="docs/images/deck-fkeys.png" width="49%" alt="Function-key Page with F1 to F12 and modifiers">
</p>

## Build from source

The packaged 2.0 app is a Windows 11 x64 MSI. Linux source builds need a desktop session and Device permissions; see [setup](docs/setup.md).

`main` holds Open TouchDeck 2.0, the native Rust and Slint app, and the Firmware. The 1.x app for Windows 10, with its Electron window and React UI, is built from the [`release/1.x` branch](https://github.com/Duresa7/OpenTouchDeck/tree/release/1.x).

- [App development](app/README.md)
- [Firmware](docs/firmware.md)
- [Layout format](schema/README.md)
- [Packaging and releases](docs/packaging.md)
- [Privacy and update checks](docs/PRIVACY.md)
- [Coding standards](CODING_STANDARDS.md): read before implementing or reviewing changes.
- [Project wiki](docs/wiki/index.md): read before revisiting a decision or recurring problem; entries record dated history.

## License

Open TouchDeck is licensed under the [Apache License 2.0](LICENSE).

## Code signing policy

The Windows programs and installers are not Authenticode-signed yet. The [code signing policy](docs/code-signing.md) says what will be signed, who approves it, where signed builds come from and what the app sends over the network.

## Icon sources

The bundled catalog in [`td-host`](app/crates/td-host/src/catalog/index.json) carries icons from [Simple Icons](https://simpleicons.org) and [selfh.st/icons](https://selfh.st/icons) by selfh.st. Simple Icons uses CC0 1.0 for most icons, with individual licenses and attribution links in the catalog. selfh.st icons use [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Icons are resized, padded or tinted to fit a Tile, so they are modified from the originals. Logos are trademarks of their owners. Open TouchDeck is not affiliated with or endorsed by them.

The Companion also supports curated [Iconify](https://iconify.design) Icon packs from the npm registry (SVG Logos, Devicon, Skill Icons, Font Awesome 6 Brands, Material Design Icons, Tabler Icons and Lucide), custom Iconify JSON files and Brandfetch logos. In 2.0.0, **Integrations > Icon packs** and **Integrations > Brandfetch logos** are placeholder pages; their 2.0 UI and the Layout editor's icon picker land in 2.0.1.

The app uses [Lucide](https://lucide.dev/license) interface icons and the [Archivo](https://github.com/Omnibus-Type/Archivo) font. See [fonts and licenses](docs/design.md#fonts-and-licenses) and the [icon-source research](docs/research/icon-sources.md).
