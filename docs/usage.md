# Open TouchDeck user guide

Design a Layout in the app, send it to a Device and tap its Tiles to use it. A Layout holds Pages; each Page holds Tiles. An Integration supplies live data or controls on this PC. The Companion runs in the tray and connects everything over USB.

This guide covers the 1.0 app. App screenshots use demo data at the default 1100 × 760 window size.

## Install

You need a Waveshare ESP32-S3-Touch-LCD-5B, a USB data cable and Windows x64.

1. Download [OpenTouchDeck-windows-x64-setup.exe](https://github.com/Duresa7/OpenTouchDeck/releases/latest/download/OpenTouchDeck-windows-x64-setup.exe) from the project's [releases](https://github.com/Duresa7/OpenTouchDeck/releases/latest).
2. Run the installer. It installs for the current user, without administrator rights. No development tools are needed.
3. Open **Open TouchDeck** from the Start menu.

Windows SmartScreen may warn because the installer has no Authenticode signature. Check that the download came from the project's release before choosing **More info > Run anyway**. The built-in updater checks a separate project signature.

The [packaging guide](packaging.md) lists installed files and uninstall behavior. Developers and Linux users should follow [setup](setup.md) and [app development](../app/README.md).

### Uninstall

Uninstall from Windows **Installed apps**. The uninstaller keeps Layouts and settings unless you choose to delete them. Its prompt asks: “Also delete your Open TouchDeck Layouts and settings, including Integration settings, from this PC?” Choosing **Yes** removes the app's local data. It does not erase the Layout or Assets on the Device, or exported files you saved elsewhere.

Integration secrets and the Brandfetch API key stay in the system keychain either way: Windows Credential Manager on Windows, GNOME Keyring or KWallet on Linux. Before uninstalling, use **Forget** beside each saved secret in **Integrations** if you want to remove it too. A later reinstall turns **Start at sign-in** back on.

## First run and connection

1. Plug the Device's USB-Serial/JTAG port into this PC with a data cable. Close any serial monitor or flash tool using it.
2. Wait for its Home card to say **Connected**. There is no manual Connect step.
3. If setup opens, enter a name such as “Desk”. The name stays on the Device.
4. Choose **Landscape** or **Portrait**. Each orientation has its own Layout. A change may restart the Device.
5. Choose **Starter Layout**, **Blank Layout** or **Keep** the existing Layout when offered.
6. If a Firmware update is offered, choose **Update** and keep USB connected, or **Skip** to continue with the current Firmware.
7. Choose **Edit Layout** to customize it, or **Done** to return Home.

Click the Device on Home to edit it. The navigation rail down the left edge of the window opens Home, Layout, Integrations and Settings; hover an icon to see its name. Settings is a page with its categories in a column beside the rail: **General** (**Appearance** and **Start at sign-in**), one entry per Device under **Devices**, then **Paired browsers**, **Troubleshooting**, **Errors** and **About**.

Closing the window leaves the Companion running in the tray. **Open** brings the window back; **Quit** stops it. Keep the Companion running for keyboard input, commands, Volume, Media and live readings. Keyboard Tiles send input to the focused window on this PC. The Device is not a standalone USB keyboard.

## Install Firmware on a New board

A supported board without working Open TouchDeck Firmware appears as **New board**.

1. Choose **Install Firmware** on its card.
2. Read the warning. Installation replaces everything on the board, including another project. Cancel if you need to preserve it.
3. Confirm the target and bundle in the native confirmation dialog on this PC. A browser cannot give this approval.
4. Keep USB connected while **Installing Firmware…** is shown.
5. After **Firmware installed**, choose **Next** and follow the naming and Layout steps above.

**Ignore** leaves the board unchanged. Select its ignored card later to install. If installation fails, read **Details**, unplug and reconnect the board, then retry. The installer includes the Firmware bundle; no separate Python or flashing tool is needed for this flow.

## Edit a Layout

![Layout editor in light mode with demo Pages](images/app-layout-light.png)

1. Open a Device from Home. At **Start a Layout**, choose **Load from Desk** (using the Device's name), **Starter Layout**, **Blank Layout** or **Import**.
2. Select a Page below the preview. **+ Page** adds another Page.
3. Select a Tile to edit its label, icon, color, size and action in the right panel. **Add Tile** or an empty cell opens the choices for action Tiles and Preset tiles.
4. Drag Tiles to move them. Use the size controls to resize them. Changes that overlap another Tile or go outside the grid are refused.
5. Click away from a Tile to edit the Page name and grid. Pick the grid by sweeping over the squares, as you would to insert a table, and click the size you want. **Full screen** hides the Page name and dots; swiping still changes Pages. The trash can at the bottom of the panel removes the Page after a second click.
6. Choose **Send to Device**. It turns into a spinner while it sends, says **Saved** when the Device has the Layout, then folds away until you change something. Wait for **Saved** before unplugging. To send again without a change, choose **Send to Desk** in the Layout options menu.

The panel button at the right of the header hides the details panel, so the Device can grow into the room. Selecting a Tile or choosing **Add Tile** brings the panel back.

The pencil beside the Device's name at the top left renames it. Press Enter to save the name to the Device, or Escape to keep the old one. Its other settings are in **Settings**, under its name in **Devices**.

The orientation buttons in the editor choose which Layout to edit. They do not turn the Device. To change how it stands, open its page in **Settings**, change **Orientation** and confirm **Restart**. Send the other orientation's Layout separately.

Choose **Preview** or press **P** to try Page swipes, Tile presses and Page jumps. Arrow keys change Pages. **Actual size** shows Device pixels at their original size. Preview does not run commands on this PC.

### When controls need Send to Device

![Controls review in light mode with a demo Volume and Media Layout](images/app-controls-light.png)

After an upgrade, reinstall or move to another PC, a Device can show live values while its Volume and Media controls do not respond. Home keeps **Connected** and adds **Controls need Send to Device**. The same notice appears in the editor if this PC has no sent copy, or the Device's Layout differs from that copy.

1. Choose **Review Layout**. If the editor shows the other orientation, use **Switch to Landscape** or **Switch to Portrait** first.
2. If you have an unsent draft, choose whether to replace it with the Device's Layout or **Keep mine**. Export a draft first if you need both.
3. Click away from a Tile to see **Controls** in the Page inspector. It lists control Tiles on every Page, their Integration and account and what each control does. Select a row to edit that Tile. Resolve any **Unknown Integration** or **Unknown account** before sending.
4. Choose **Send to Device** when the Layout is the one you want. Sending replaces the Layout on the Device and lets its Tiles control the Integrations shown here.
5. Wait through **Checking controls…**. The notice clears after the Companion checks the Device's Layout, not just when the transfer finishes. If it cannot check, follow the message to reconnect the Device or update its Firmware. A failed send keeps the guidance, and its error card's **Details** opens **Settings > Errors**.

Loading or reviewing a Layout does not authorize its controls. Repeat for each Device and orientation you use.

### Actions and approvals

Under **When tapped**, choose a key or shortcut, media key, modifier, app launch, file or website open, command, Page jump or no action. **When held** gives an action a separate hold behavior where offered.

For a command, enter the program, arguments and working folder in the action form. Check each argument. Shell mode does not turn the program field into a general shell script.

The first attempt to start an unapproved program opens a native dialog on this PC. This applies to commands, app launches and file or folder opens; ordinary HTTP, HTTPS and mail links do not need process approval. Review the exact program, arguments, working folder and shell setting, then choose **Run once**, **Always allow** or **Deny**.

**Always allow** remembers that exact command on this PC. Changing its arguments or other approved fields asks again. A Layout imported from someone else cannot approve itself. If the app says **Waiting for approval on this PC**, find the native dialog or use **Show prompt again**. Device key input pauses while a native approval dialog is open.

### Icons, Assets and backups

Use the Tile's **Icon** controls to choose a built-in symbol, search the catalog, upload a PNG, JPEG or SVG image, or use an app's icon where offered. Search uses the bundled catalog and every installed Icon pack; picking an uncached catalog image needs internet access, while pack icons draw offline. The Companion prepares it as an Asset for the Device.

For more icons, choose **Get more icons** in the picker, or open **Settings > Icon packs**. Download a curated pack such as SVG Logos or Material Design Icons, or choose **Import a pack file** for an Iconify JSON file of your own. Each pack shows its license and size; **Remove** deletes it from this PC. Packs whose license asks for credit are named in the picker's credits.

To search brand logos as well, turn on [Brandfetch logos](#brandfetch-logos) with your own keys.

Drafts are saved as you edit, in this window or browser's local storage, separately for each Device and orientation. Closing the app keeps them, and nothing reaches the Device until you send. Export files for backups and transfers between browsers or PCs.

The **Layout options** menu (three dots) offers **Send to Desk**, **Load from Desk**, **Export**, **Import**, starting Layouts and **Remove unused Assets**. Export both orientations before replacing a Layout. Open the exported files again to check them.

A Layout JSON file contains Asset references, not the image files, Integration secrets or command approvals. Keep original custom images and GIFs separately. On another PC, set up Integrations again, check program paths and prepare missing Assets before sending. Loading a Layout from the Device does not copy its Assets back to this PC. Remove unused Assets only after keeping the originals you need.

### Brandfetch logos

The icon picker can also search [Brandfetch](https://brandfetch.com) for brand logos. It's off until you add your own Brandfetch keys. Open TouchDeck ships none, and Brandfetch's terms ask each user to use their own account.

1. Register at the [Brandfetch developer portal](https://developers.brandfetch.com/register). Copy your **client ID** and your **API key**.
2. Open **Settings > Brandfetch logos**.
3. Paste the client ID into **Client ID** and choose **Save**. The window reloads so it may reach Brandfetch, and comes back to the same Settings. The icon picker now shows a **Brandfetch** section with up to five results as you type.
4. Paste the API key into **API key** and choose **Save**. It shows **Set**. Choose **Test key** to check it.
5. In the icon picker, type a brand name and choose a Brandfetch result. The Companion fetches its logo and prepares it as an Asset, like any other icon.

**How the key is stored.** The API key goes to the system keychain: Windows Credential Manager on Windows, GNOME Keyring or KWallet on Linux. Open TouchDeck never shows it again, never writes it to a file and never sends it anywhere but Brandfetch's Brand API. If Linux has no keychain running, saving says "No system keychain is available. Start GNOME Keyring or KWallet and try again." Start one, then save again. **Details** under the message shows the Companion's own words. The client ID isn't secret: it's part of every Brandfetch image address.

**What it costs.** Searches use your client ID and are free. Each pick uses one Brand API request, and so does each refresh below. The free developer plan includes about 100. **Test key** asks about Brandfetch itself, which Brandfetch doesn't count.

**The 30-day refresh.** Brandfetch lets an app keep a logo for 30 days. From day 25 the Companion fetches each Brandfetch logo again. It checks at start and every hour, and makes at most 10 Brand API requests a run; the rest wait for the next hour. If a logo changed, the Companion prepares the new one as a new Asset, and the Layout editor puts it in your drafts in place of the old one. Send the Layout to show it on the Device.

A refresh that fails because Brandfetch didn't answer tries again the next day. If Brandfetch refuses the key, refreshes stop until you save a key again. If Brandfetch no longer knows a brand or has no logo for it, that logo isn't asked for again. A logo that can't be fetched for 30 days loses its cached copy on this PC, and **Settings > Brandfetch logos** says how many. It's never asked for again; Tiles keep showing it, and you can pick it again to start over. Logos whose Asset you removed from this PC are forgotten without a request.

Searches and their results aren't kept. Recent icons remember only the prepared Asset. **Forget** beside the client ID turns search off and reloads the window; **Forget** beside the key deletes it from the keychain. Each asks first.

## Set up Integrations

![Integrations in dark mode with demo data](images/app-integrations-dark.png)

Open **Integrations**, select one and turn it on. Choose **Save** for configuration changes. Secrets have their own **Save** and **Forget** controls. Use **Check now** when available. **Check started** means the request was queued; wait for the status or readings to change, then inspect **Details** if it cannot connect. A slow Integration does not hold up checks for the others.

Turning an Integration off makes its Live tile readings on a connected Device stale within about a second. The last reading can remain visible, dimmed with an unavailable marker. Volume drags stop changing this PC while Volume is off. Turn the Integration on again and wait for fresh readings to restore the Tiles.

Choose a Preset tile's **Add to Layout**, select its destination when asked, then open Layout and **Send to Device**. Adding a Preset tile edits the Layout; it does not send it automatically.

### This PC

Turn on **This PC** and save. It needs no account or key. Add its CPU, memory or network Preset tiles. For a Graph Tile, select the value, display style, history and accent in the inspector. A graph needs enough room; unavailable sizes are disabled.

### CLIProxyAPI usage limits

Use your own CLIProxyAPI hub and its management key. Open TouchDeck does not sign in to AI providers for you.

1. Set up the hub and sign the desired accounts into it. Enable its management API.
2. In **Integrations > CLIProxyAPI**, turn it on. Replace the placeholder `https://YOUR_HUB_URL` with your hub's address.
3. Set **Check every** and choose **Save**. The default is 120 seconds; polling cannot be faster than 60 seconds.
4. Paste your own `YOUR_MANAGEMENT_KEY` into **Management key** and use its **Save** button. Do not put the key in a Layout or share it in a screenshot.
5. Choose **Check now**. Check the account table before adding a Preset tile.
6. Add a usage Tile, select its account and value, then **Send to Device**.

Use HTTPS for a remote hub. Plain HTTP on a private network exposes the key in transit; public HTTP is refused. Keys are stored in the system keychain: Windows Credential Manager on Windows, GNOME Keyring or KWallet on Linux.

Codex and Claude readings include 5-hour and 7-day usage limits and resets when available. Other providers can supply account status and request counts. An unavailable limit is not zero. **All accounts** uses the highest usage percentage for each window. Check-now requests share a cooldown. After repeated failed checks, readings become stale and Tiles grey out.

Use **Forget** beside the key to remove it. Uninstalling the app does not remove Integration secrets from the keychain.

### Custom HTTP values

**Custom** reads numbers or text from an HTTP response or an approved command. Its **Custom sources (JSON)** field holds a list, with at most 16 entries. Each entry needs a unique `id`, a `name`, `interval_s` (1 to 3600), a `kind` (`number` or `text`) and an `input`.

For an HTTP response such as `{"temperature": 21.5}`, adapt this example. Replace the address before saving:

```json
[
  {
    "id": "temperature",
    "name": "Temperature",
    "interval_s": 30,
    "kind": "number",
    "input": {
      "type": "http",
      "url": "https://YOUR_SERVICE_URL/metrics",
      "value_path": "$.temperature",
      "headers": {}
    }
  }
]
```

Turn on Custom and choose **Save**, then **Check now**. Paths start at `$`; use `.field` for an object field or `[0]` for an array element. Wildcards are not supported. The selected value must match `kind`.

For authentication, map an HTTP header to a secret-field name, for example `"headers": {"Authorization": "metrics_token"}`. Save the configuration, then enter the complete header value in the generated secret field and save it separately. Keep secret values out of the JSON. Changing the service address may require saving its secret again.

Numeric entries offer value and graph Preset tiles. Add one and send the Layout.

### Custom command values

For a command, use the same outer entry and replace `input` with:

```json
{
  "type": "command",
  "program": "C:/Tools/YOUR_PROGRAM.exe",
  "args": ["--print-value"],
  "value_path": null
}
```

Replace the program and arguments with a program you trust. Use an absolute executable path; Windows commands must use an `.exe` or `.com` program. With `value_path: null`, the program's output is read as the selected number or text. If it prints JSON, use a path such as `$.value` instead.

Save and check the Integration. Approve the command in the native dialog on this PC. **Run once** allows one execution; a polling command needs **Always allow** to keep running without new approvals. **Deny** leaves it unable to run. Editing the command asks again. Command output must be small and finish promptly; failures appear in the Integration's status.

### Volume

Turn on **Volume**, save and add **Volume bar**. Send the Layout, then drag along the bar on the Device to change this PC's default output volume. Tap the mute glyph to mute or unmute. Its live level follows changes made on this PC too. Controls wait for a fresh reading after reconnection; taps from the previous connection are discarded. It needs an available audio output and current Firmware.

### Media

Turn on **Media**, save and add **Now playing**. Start playback in an app that exposes a Windows media session, then send the Layout. The Tile shows title, artist, cover art and playback state when the player supplies them. Its control parts are previous, play/pause and next. Tap the relevant control on the Device. A player may not support every control or supply cover art. The Tile is not a seek slider.

## Animated GIF Tiles

Choose **Add Tile > GIF**, select a file and choose **Whole GIF** or **Fill Tile**. Wait for preparation to finish, inspect the fitted dimensions and timing, then **Send to Device**. **Change GIF** replaces it later.

The Companion resizes and resamples the animation for the Device; the Firmware plays it from stored Assets. Current Firmware allows up to 128 × 128 pixels per GIF, 60 frames, at least 100 ms per frame (10 frames per second), and four animated Tiles per Page. The per-Page pixel budget is 65,536; total decoded frame pixels per GIF are capped at 983,040. Prepared Assets must also fit the 262,144-byte Asset limit. The app checks the connected Device's reported limits, so older Firmware may offer less or require an update.

If preparation fails, shorten the animation, reduce its dimensions or use fewer animated Tiles. Resizing a Tile can require preparing its GIF again. Keep the original GIF for later edits and backups.

## Photo frame and idle screen

Open the Device's page in **Settings**. Turn on **Photo frame**, set **Start after** and use **Choose folder** in the desktop app. Pick a folder of pictures on this PC. Folder selection is not available in the browser.

When idle, the Device shows pictures sent by the Companion, one at a time. They are not kept on the Device. Without the Companion, the idle screen shows a clock. Touch the screen to return to the Layout. Change **Brightness** on the same page. If a field says it needs newer Firmware, update before using it.

## Use the browser version

Keep the Companion running on this PC. Choose **Open in browser** from the tray menu. The default browser opens and pairs automatically. If it asks for a code, enter the code shown on this PC within two minutes. For another browser, use the same address and a fresh pairing code.

Pairing survives refreshes. After the Companion restarts, choose **Open in browser** again. **Settings > Paired browsers > Forget** signs that browser out, including its other tabs. Other paired browsers stay connected.

Browser access is local to this PC, not available to a phone or another PC on the network. Command and Firmware approvals still happen in a native dialog on this PC. Choose photo folders and install app updates from the desktop app.

## Updates and Firmware rollback

### Update the app

1. Export any Layout drafts you want to keep.
2. Open **Settings > About > Check for updates**, or the **Update available** icon at the bottom of the navigation rail.
3. Review the release notes and choose **Update**. You can keep working during the download.
4. At **Ready to restart**, choose **Restart**. Wait for any Device transfer to finish first.

The app verifies the update signature before running the installer. If verification fails, leave the current version installed and retry later. The update keeps user data and restarts Open TouchDeck. It does not update Device Firmware.

### Update Firmware

Open the Device's page in **Settings**. If newer Firmware is offered, choose **Update** under **Firmware**. Keep USB connected until the result is known. The page shows these phases:

1. **Checking the file…** validates the bundle.
2. **Updating…** with a percentage sends the Firmware.
3. **Restarting…** waits for the Device to restart.
4. **Checking that it started…** checks the Firmware that came back.
5. **Updated to** the version confirms success. A rollback or failure shows its own result instead.

If the same Firmware is already installed, the result says **Already on Firmware** with its version. No transfer or restart is needed.

To use a release's `.tdfw` bundle, open **Settings > Troubleshooting > Update from a file** and choose the file for the intended Device. Use a bundle for the supported board, not the desktop installer. A downgrade requires confirmation in the native dialog on this PC.

Normal updates write the inactive Firmware slot. If the new Firmware fails to confirm a successful start, the Device can roll back to the previous slot. Read the result after the restart: reaching 100% transfer does not prove the new Firmware started. If the app reports rollback, the previous Firmware still runs; check **Details** before retrying.

### Upgrading from 0.2

Install Open TouchDeck by hand from the release. It installs next to TouchDeck 0.2 and does not remove it. There is no automatic bridge from 0.2, and its app Layout drafts and settings do not carry over.

1. In TouchDeck 0.2, export your Layouts and keep your original Assets.
2. Close TouchDeck 0.2. Only one app can use the Device at a time.
3. Start Open TouchDeck. Load the Device's Layout or import an exported file, then set up Integrations again.
4. When you no longer need it, uninstall **TouchDeck** from Windows **Installed apps**. This leaves Open TouchDeck and its start at sign-in in place.

**After updating, Volume and Media control Tiles respond only after you send their Layout from the app once. Choose Send to Device.** The Companion acts only on a Layout it pushed itself. Follow [When controls need Send to Device](#when-controls-need-send-to-device) to review the Tiles before sending. Repeat for each Device and orientation you use.

App launches and file opens can now ask for command approval on this PC too.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| No Device on Home | Use a USB data cable and the USB-Serial/JTAG port. Unplug and reconnect. Linux source builds also need serial permissions from the [setup guide](setup.md). |
| In use by another program | Close serial monitors and flash tools. The Companion reconnects automatically. |
| Not connected after the Device drops the link | The Companion reconnects automatically after the Device drops its session, even if USB stays plugged in. Controls wait for the new connection and fresh readings. Repeated drops make retries less frequent. If it stays disconnected, check USB and open **Settings > Troubleshooting** for the last error. |
| Couldn't send a Layout, or an Asset operation failed | Choose **Details** on the error card to read the full error in **Settings > Errors**, then reconnect and retry. A stalled transfer or an unreadable Asset list shows an error. Check that Tiles and Assets fit the Device's capabilities. |
| Needs newer Firmware | Update from the Device's page in **Settings**. Older Firmware cannot draw every Live tile or animation. |
| Controls need Send to Device | Choose **Review Layout**, check the Controls list and **Send to Device**. See [the recovery steps](#when-controls-need-send-to-device). |
| Volume or Media does not respond | Enable the Integration and check the control notice. For Media, start a supported player; for Volume, check the default audio output. After reconnection, wait for fresh readings. |
| Waiting for approval on this PC | Find the native dialog. Check the exact command before approving. |
| A Live tile is blank or grey | Keep the Companion running. Check the Integration's status, credentials and **Details**, then retry its check. |
| Photo folder is missing | Choose an existing folder again in the desktop app. |
| Pairing code is wrong or expired | Choose **Open in browser** from the tray again for a new code. |
| Firmware update failed | Keep the Device plugged in until the result is known. Check whether it rolled back, then read **Details** before retrying. |

When something fails, a card pops up at the bottom of the window with what went wrong. Its **Details** opens **Settings > Errors**, which lists the errors since Open TouchDeck opened, newest first, with the full detail and a button to copy it.

**Settings > Troubleshooting** includes Device details and a Firmware console. Overlong console lines are truncated and heavy output can drop lines so Device replies keep flowing. Logs can contain personal data; remove it before sharing. If the Device's Layout prevents normal use, its ten-second touch hold starts the built-in Layout without removing stored files. See [Firmware recovery](firmware.md#recovery) before using it.

For shortcut and media-key Tiles on a Device that offers a Bluetooth keyboard,
choose **Send to → Bluetooth computer or phone** in the Tile action form. Turn
on the Bluetooth keyboard and add the computer or phone in Device settings first.
These Tiles work without the Companion. **This PC (Companion)** keeps sending
keys to the PC running the Companion. A disconnected Bluetooth destination drops
the press; it does not send it to the Companion instead. Firmware without the
Bluetooth keyboard refuses a Layout with Bluetooth Tiles, so before you switch a
Device back to the normal build, send those Tiles to **This PC (Companion)**;
otherwise the Device starts its built-in Layout.
