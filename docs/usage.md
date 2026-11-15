# Open TouchDeck user guide

Design a Layout in the app, send it to a Device and tap its Tiles to use it. A Layout holds Pages; each Page holds Tiles. An Integration supplies live data or controls on this PC. Open TouchDeck 2.0 is one native program (`td-app.exe`) that runs the window and the Companion in the same process.

## Install

You need a Waveshare ESP32-S3-Touch-LCD-5B, a USB data cable and Windows 11 x64. Windows 10 users stay on the 1.x installer on [Releases](https://github.com/Duresa7/OpenTouchDeck/releases).

If Open TouchDeck 1.x is installed on this PC, replace it by hand first ([ADR 0013](adr/0013-replace-1x-by-hand-for-2x.md)):

1. Quit Open TouchDeck.
2. Uninstall Open TouchDeck 1.x from Windows **Installed apps**. When asked whether to delete your Layouts and settings, choose **No**.

Then install 2.0:

1. Download `OpenTouchDeck-2.0.0-windows-x64.msi` from the [latest release](https://github.com/Duresa7/OpenTouchDeck/releases/latest).
2. Run the MSI. It installs for the current user without administrator rights.
3. Open **Open TouchDeck** from the Start menu.

Windows SmartScreen may warn because the installer has no Authenticode signature yet. Check that the download came from the project's release before choosing **More info > Run anyway**.

The [packaging guide](packaging.md) covers the MSI and updater verification. Developers and Linux users should follow [setup](setup.md) and [app development](../app/README.md).

### Uninstall

Uninstall from Windows **Installed apps**. Removing the 2.0 MSI deletes the installed program and its startup entries while keeping your Layouts, settings and Integration secrets on this PC. Before uninstalling, use **Forget** on each saved secret in **Integrations** if you want to remove it from Windows Credential Manager too.

## Window, sections and tray

The left rail opens **Home**, **Layout**, **Setup**, **Settings**, **Integrations** and **About**.

On Windows, closing the window leaves Open TouchDeck running in the notification area so your Device's keys, actions and Live tiles keep working. The first time you close the window, a notice offers **Keep running** and **Quit**. Left-clicking the tray icon opens the window again; its menu has **Open**, **Start at sign-in** and **Quit**. On Linux there is no tray icon yet, so closing the window quits the program.

## Home and Setup

Plug the Device's USB-Serial/JTAG port into this PC with a data cable and close any serial monitor or flash tool using that port.

**Home** lists your known Devices and each Device's connection state (**Connected over USB**, **Connected over Wi-Fi**, **Connecting…**, **Updating…**, **In use by another program** or **Not connected**). There is no manual connect button; the Companion connects automatically.

### Install Firmware on a New board

A connected board without Open TouchDeck Firmware appears in **Setup** as **New board found**.

1. Open **Setup** and choose **Install Firmware** on the board's card.
2. Confirm the port and Firmware bundle in the native Windows dialog on this PC. Installation replaces everything on the board, including any other project.
3. Keep USB connected while **Installing Firmware…** runs. When the board restarts with Open TouchDeck Firmware, setup continues to the Device steps below.

Choose **Ignore** to leave another ESP32-S3 board alone. You can restore it later under **Ignored New boards** at the bottom of **Setup**.

### Set up a Device

When a connected Device has not been set up on this PC yet, **Setup** opens its setup steps automatically (or choose **Set up** on its card in **Setup**):

1. **Update Firmware**: if the app carries newer Firmware than the Device runs, choose **Update** and keep USB connected, or **Skip**.
2. **Name this Device**: enter a name such as `Desk` and choose **Next**. The name is stored on the Device.
3. **How does it stand?**: choose **Landscape** or **Portrait**. Each orientation has its own Layout. Changing orientation restarts the Device.
4. Choose **Done**.

Choosing **Close** dismisses setup for that Device and remembers the skip on this PC.

## Edit a Layout

Open **Layout** in the left rail. If more than one Device is known, pick the Device from the `<Device> ▾` menu in the header.

### Load or start a Layout

When this PC has no saved draft for the selected Device, opening **Layout** reads the Layout from the connected Device once:

- If the Device has no Layout yet, choose **Start a Layout** for the Starter Layout or **Start blank** for an empty one.
- If the Device's Layout differs from your saved draft, a prompt asks whether to **Keep mine** or **Replace** the draft with the Device's copy.
- To read from the connected Device again at any time, open **Layout ▾** in the header and choose **Read from Device**.

Drafts are saved automatically on this PC as you edit. Nothing changes on the Device until you send the Layout.

### Pages and the Device canvas

- **Pages**: the filmstrip on the left lists the Layout's Pages (up to eight), showing each Page's number, thumbnail, `Usage` tag (for a Usage Page) and title. Use **Hide Pages** or **Show Pages** in the header to fold or open the filmstrip; it folds automatically when the window is narrow. Choose **+ Add Page** to add a Page, or open a Page's `⋯` menu and choose **Remove Page**.
- **Add a Tile**: click an empty `+` cell on the Device canvas (or focus the cell and press Enter or Space) to add a Key Tile.
- **Select, move and resize**: click a Tile to select it. Drag a Tile to another cell to move it, or drag its bottom-right corner handle to change its row and column span. Moves and resizes that go outside the grid or overlap another Tile are refused.
- **Keyboard controls on the canvas**: arrow keys move between cells or nudge the selected Tile, Shift+arrow keys resize the selected Tile, Delete or Backspace removes the selected Tile, and Escape clears the selection.
- **Send to Device**: the header shows **Sent**, **Unsent changes**, **Sending…** or **Couldn't confirm**. Choose **Send to Device** to write the Layout to the connected Device and update this PC's Pushed copy so control Tiles on the Layout can act on this PC.

In 2.0.0, the Layout editor arranges Pages and the grid canvas and sends Layouts over USB. Editing a Tile's label, icon, color or action in an inspector panel, adding Preset or animated GIF Tiles, editing Page titles or grid sizes, Preview mode, and importing or exporting Layout files are not in the 2.0.0 window yet; existing Tiles read from a Device or kept in your saved drafts preserve their actions, icons and Preset bindings when you arrange and send them.

## Settings and the Device page

Open **Settings** in the left rail.

### This PC

- **Start at sign-in** (Windows): starts Open TouchDeck in the tray when you sign in to Windows.
- **Check for updates** (Windows): checks GitHub Releases after startup and about once a day for a newer stable 2.x MSI ([PRIVACY](PRIVACY.md)).
- **Appearance**: choose **Match Windows**, **Light** or **Dark**.

### Device page

Select a Device under **Devices** in **Settings** to open its page:

- **General**: edit **Name** and press Enter to save the new name to the Device. Choose **Landscape** or **Portrait** under **Orientation** and confirm **Restart** to turn the Device's screen.
- **Screen**: turn on **Photo frame**, choose the idle delay in **Start after** (`30 s` to `1 hr`), and choose **Choose folder** to pick a folder of pictures on this PC. While the Companion is connected, the Device shows pictures from that folder one at a time when idle; without the Companion, the idle screen shows a clock. Touch the screen to return to the Layout.
- **Wireless** (shown when the Device runs the radio Firmware build): lists the Device's **Paired apps** over USB and marks **This app**. Use **Forget** beside an entry to remove that Pairing, or **Forget all** and confirm to clear every paired app on the Device. Managing Bluetooth keyboard hosts from the Device page lands in 2.0.1.

### Install or update deck firmware

Under **Firmware** on a Device's page in **Settings**, the app shows the Device's current Firmware version and status (**Up to date**, `<version> available`, **Not bundled in this build** or **Not reported**), plus a **Restart** button.

- **Bundled update**: when the app carries a newer Firmware build than the connected Device runs, choose **Update** over USB and keep the Device plugged in through transfer, restart and startup verification.
- **From a file**: under **From a file**, choose **Update from a file…** and pick a `.tdfw` bundle for the Waveshare ESP32-S3-Touch-LCD-5B over USB. Every file update asks for confirmation in a native dialog on this PC, even when the file is newer. The dialog says the file isn't signed by Open TouchDeck and will replace the Device's Firmware, and shows the file's version and build next to the Device's. Cancelling leaves the Device untouched. A bundled update still asks only for a downgrade or a build switch.
- **Firmware build**: when the app carries the other Firmware build, the **Firmware build** section lets you switch the Device between the normal (USB-only) build and the radio build.

Normal updates write the inactive Firmware slot. If the new Firmware fails to confirm a successful start after reboot, the Device rolls back to the previous slot and the page reports the rollback.

## Set up Integrations

Open **Integrations** in the left rail. Each Integration row shows its status and an on/off switch:

- **This PC**: supplies CPU, memory and network readings with no account or key.
- **CLIProxyAPI**: polls AI account usage limits from your own CLIProxyAPI hub.
- **Custom**: polls numbers or text from HTTP endpoints or approved commands on this PC.
- **Volume**: supplies the default audio output level and mute state on this PC and accepts Volume bar changes from the Device.
- **Media**: supplies now-playing title, artist, cover art and playback state from the active Windows media session and accepts play/pause, previous and next controls.

Turning an Integration off makes its Live tile readings on a connected Device stale within about a second, and Volume or Media controls stop acting until you turn the Integration back on.

### Configure CLIProxyAPI and Custom

Choose **Settings** on **CLIProxyAPI** or **Custom** to open its settings page:

1. Edit the configuration fields and any secret fields, then choose **Save**. One **Save** button saves both settings and any secret you typed.
2. Under **Status**, choose **Check now** to poll immediately.

Secrets are stored in the system keychain (Windows Credential Manager on Windows, Secret Service on Linux) and are never written to a file. When a secret is already saved, choose **Replace** to enter a new one (**Keep the saved one** cancels), or choose **Forget** and confirm to delete it from the keychain.

For **CLIProxyAPI**:

1. Enter your hub's address in **Hub URL** (use HTTPS for a remote hub) and set **Check every** (at least 60 seconds).
2. Enter your hub's key in **Management key** and choose **Save**.
3. Choose **Check now** and inspect the **Accounts** section, which lists each account's **Service**, **Plan**, **5-hour limit**, **7-day limit** and **Status**.

For **Custom**:

1. In **Custom sources (JSON)**, enter a JSON array of up to 16 entries. Each entry needs a unique `id`, a `name`, `interval_s` (1 to 3600), `kind` (`number` or `text`) and an `input` (`http` or `command`).
2. For an HTTP source that needs a secret header, map the header name to a secret field name in `headers` (for example `"headers": {"Authorization": "metrics_token"}`) and choose **Save**. Then enter the secret header value in the generated secret field and choose **Save** again.

### Command approvals

The first time a Tile action or a Custom command source tries to run an unapproved program on this PC, Open TouchDeck opens a native confirmation dialog showing the exact program, arguments and working folder. Choose **Run once**, **Always allow** or **Deny**. Changing the program or its arguments asks again.

If a command is waiting for your decision on this PC, the Integration page shows `Waiting for approval on this PC` with a **Show again** button to bring the native prompt back.

### Icons section

At the bottom of **Integrations**, **Icon packs** and **Brandfetch logos** open placeholder pages in 2.0.0. Downloading or importing Icon packs and configuring Brandfetch keys in the 2.0 window land in 2.0.1.

## About

Open **About** in the left rail to see the installed Open TouchDeck version, the GPL-3.0 licence notice for the built program, the Slint credit and the alphabetical list of open-source components and their licences.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| No Device on Home | Use a USB data cable on the Device's USB-Serial/JTAG port. Unplug and reconnect. Linux source builds also need the serial and `uinput` permissions in [setup](setup.md). |
| In use by another program | Close serial monitors and flash tools using the Device's port. The Companion reconnects automatically. |
| Not connected after the Device drops the link | Keep USB connected; the Companion reconnects automatically. If it stays disconnected, unplug and reconnect the USB cable or restart the Device from its page in **Settings**. |
| Waiting for approval on this PC | Find the native confirmation dialog on this PC, or open the Integration's settings page and choose **Show again**. Check the exact program and arguments before approving. |
| A Live tile is blank or grey | Keep Open TouchDeck running, check that the Integration is turned on in **Integrations**, and use **Check now** on its settings page. |
| Volume or Media does not respond | Turn on the **Volume** or **Media** Integration and send the Device's Layout from **Layout > Send to Device** so this PC has the Pushed copy. |
| Photo frame does not start | Open the Device's page in **Settings**, check that **Photo frame** is on and use **Choose folder** to select an existing folder with images. |
| Firmware update failed | Keep the Device plugged in until the status settles. Check whether the Device rolled back to its previous Firmware, then retry from the Device's page in **Settings**. |

If a Layout on the Device prevents normal use, hold one finger on the touchscreen for ten seconds to start the built-in Layout without erasing stored files. See [Firmware recovery](firmware.md#recovery).
