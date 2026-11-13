# Copy guide

Every word in the Open TouchDeck UI follows this guide. It has the voice rules, the words to use, the key strings for the new screens, and a table that maps every string in today's app to its new form.

UI tickets copy strings from here. When a screen needs a string this guide doesn't have, write it by these rules and add it here in the same PR.

## Voice rules

1. Use plain verbs, sentence case and active voice. "Send to Device", not "Submit" or "Layout Upload".
2. A button says exactly what happens. The same action keeps the same name through a flow: "Update" leads to "Updating…" and then "Updated".
3. Errors say what happened and how to fix it. They don't apologize and they aren't vague. "Couldn't send to Desk. Check that it's plugged in, then retry." Never "Something went wrong" on its own.
4. Empty states invite an action. They say what to do next, not that nothing is there.
5. No all-caps labels. No ellipsis except on in-progress states ("Updating…"). Use the single character `…`, not three dots.
6. Use glossary terms only (`GLOSSARY.md`). The UI says Device, New board, Firmware, Companion, Layout, Page, Tile, Live tile, Preset tile, Asset and Integration, and never the words the glossary lists under "Avoid".
7. Glossary terms keep their capital letter anywhere in a sentence, as in the glossary, the spec and ADR 0004: "Add Tile", "Needs Firmware 1.4", "Install Firmware".
8. Prefer the Device's name to the word "Device". "Sent to Desk." In this guide `{device}` stands for the name. Say "the Device" only where no name exists yet.
9. Say "this PC", not "this computer". Say "Open TouchDeck", never "TouchDeck" alone. Wire names such as `touchdeck` and `.tdfw` appear only in Details (ADR 0003).
10. Shorten wherever the meaning survives. Cut "your" when the owner is obvious, cut filler such as "right on", "exactly as", "now", and drop hints that repeat their label.
11. Write lists without the serial comma ("CPU, RAM and disk"), as the spec does.
12. Use straight apostrophes in code and curly quotes “ ” around names the user typed, such as a Layout name.
13. Raw technical messages from the Companion or the Firmware never stand alone. Show a sentence from this guide and put the raw message under "Details".

## Words

| Say | Instead of | Where it applies |
| --- | --- | --- |
| Device, or the Device's name | deck, your deck, TouchDeck (the hardware), practice deck | Everywhere |
| New board | blank deck, recovery deck, port that isn't a TouchDeck | A connected board without Open TouchDeck Firmware |
| Firmware | deck software, software, the new software | The program on a Device |
| Layout | buttons, layout (lowercase), config, profile | All Pages of one Device |
| Page | page (lowercase), screen | One screen of Tiles |
| Tile | button, key, slot | One slot on a Page |
| Live tile | widget, AI usage button | A Tile fed by an Integration |
| Preset tile | widget | A ready-made Tile an Integration offers |
| Asset | picture, icon file, resource | An icon, image or animation stored on a Device |
| Icon | picture, symbol | The picture on a Tile, in the picker |
| Integration | source, AI usage, hub (as a page name) | A connection that supplies data or takes commands |
| CLIProxyAPI | AI usage | The Integration that reads AI usage limits |
| Usage page | AI usage page, dashboard | The CLIProxyAPI Page kind |
| Update | version updates, download update, install | App and Firmware updates |
| Companion | daemon, service, backend, host | The background program, named only in errors and the tray |
| this PC | this computer, your computer | The PC the Companion runs on |
| Tap | press, click (for the Device) | Touching a Tile on the Device. "Click" stays for the mouse in the app |
| Retry | try again (as a button) | Buttons. Sentences may still say "then retry" |
| Forget | clear, delete (for secrets and pairings) | Removing a stored key, a paired browser, a Wi-Fi network or a Keyboard host |
| Keyboard host, or "computer or phone" in sentences | bond, paired device, Pairing | A computer or phone the Device's Bluetooth keyboard types on. It may be this PC, but the copy doesn't assume so |
| Add (a computer or phone) | pair, bond (for the Bluetooth keyboard) | Making a Keyboard host. "Pair" stays for a Pairing with this app over Wi-Fi |
| Remove | delete | Removing a Tile, a Page or unused Assets |

## Patterns

**Action, progress, result.** Every action that takes time names one verb three ways.

| Button | In progress | Done | Failed |
| --- | --- | --- | --- |
| Update | Updating… {pct}% | Updated to {version} | Couldn't update. {fix} |
| Install Firmware | Installing Firmware… {pct}% | Firmware installed | Couldn't install Firmware. {fix} |
| Send to Device | Sending… | Saved on the button, then it folds away; Sent to {device} in the status line | Couldn't send to {device}. {fix} |
| Load from {device} | Loading… | Loaded “{layout}” from {device} | Couldn't load from {device}. {fix} |
| Restart | Restarting… | (the Device's status returns to Connected) | Couldn't restart {device}. {fix} |
| Repair | Repairing… {pct}% | Repaired | Couldn't repair {device}. {fix} |
| Import | Importing… | Imported “{layout}” | “{file}” isn't a Layout file. Nothing changed. |
| Export | (none, it's instant) | Exported “{file}” | Couldn't export. {fix} |
| Check for updates | Checking… | Up to date, or the Update icon appears | Can't reach the update server. {fix} |
| Pair | Pairing… | Paired | That code didn't match. {fix} |
| Save | Saving… | Saved | Couldn't save. {fix} |

**Errors.** Start with "Couldn't {verb}" or with the fact ("{device} didn't answer."). Then give one fix as an instruction. Put the raw reason under "Details".

**Fixes for a Device.** Pick the first that applies.

1. "Check that it's plugged in, then retry."
2. "Unplug it and plug it back in."
3. "Repair it from Settings, Troubleshooting."

**Confirmations.** The title asks the question with the verb. The confirm button repeats the verb. The other button says "Cancel". Destructive confirms say what is lost.

**Numbers and units.** Write "{n} s", "{n} min", "{n} hr", "{n} KB", "{n} MB" and "{n}%". Use digits. Pluralize with the count ("1 Tile", "2 Tiles").

**Tooltips.** Name the control in two words or fewer, then a shortcut in parentheses if there is one: "Preview (P)". A tooltip never repeats a visible label.

## New screens

### Home

| Element | String |
| --- | --- |
| Greeting, 05:00 to 11:59 | Good morning |
| Greeting, 12:00 to 17:59 | Good afternoon |
| Greeting, 18:00 to 04:59 | Good evening |
| Card status, live | Connected |
| Card status, unplugged or not answering | Not connected |
| Card status, opening the port | Connecting… |
| Card status, Firmware update running | Updating… {pct}% |
| Card status, Firmware update running, no progress yet | Updating… |
| Card status, restarting | Restarting… |
| Card badge, newer Firmware exists | Firmware update |
| Card, a port another program holds | In use by another program |
| Card hint for "In use by another program" | Close the other program. {device} connects on its own. |
| Card for a New board | New board |
| Card name, a Device not named yet | Unnamed Device |
| Device list label | Devices |
| Card tooltip | Edit Layout |
| Device screen, connected with no stored Layout | No Layout yet |
| Device screen hint, no stored Layout | Click to make one |
| Empty state title | Plug in your Device |
| Empty state body | Connect an Open TouchDeck to this PC with a USB data cable. |
| Empty state button | Connection help |
| "Connection help" target | Opens Settings, Troubleshooting |
| Add card | Add Device |
| Add card, after a click | Plug the new Device into this PC with a USB data cable. Setup starts on its own. |

### Onboarding

Shown the first time a Device with Open TouchDeck Firmware connects to this PC with no name.

| Step | Element | String |
| --- | --- | --- |
| All | Title | Set up your Device |
| All | Step counter | Step {n} of {total} |
| All | Buttons | Back, Next |
| All | Close button label | Close |
| Name, Orientation | Next, while saving | Saving… |
| Orientation | Next, while {device} restarts | Restarting… |
| Starting Layout | Next, while reading the Layout on the Device | Loading… |
| Starting Layout | Next, while sending the Layout | Sending… |
| Name | Heading | Name this Device |
| Name | Field label | Name |
| Name | Placeholder | Desk |
| Name | Hint | Stored on the Device, so it keeps its name on any PC. |
| Name | Error, empty | Enter a name. |
| Name | Error, too long | Names can be up to {max} characters. |
| Orientation | Heading | How does it stand? |
| Orientation | Options | Landscape, Portrait |
| Orientation | Hint | Each orientation keeps its own Layout. You can change it later in Settings. |
| Starting Layout | Heading | Pick a starting Layout |
| Starting Layout | Option | Starter Layout |
| Starting Layout | Option hint | Media controls and everyday shortcuts, ready to change. |
| Starting Layout | Option | Blank Layout |
| Starting Layout | Option hint | One empty Page with room for 12 Tiles. |
| Starting Layout | Option, the Device already has a Layout | Keep “{layout}” |
| Starting Layout | Option hint | The Layout on the Device now: {n} Pages, {n} Tiles. |
| Firmware | Heading | Update Firmware |
| Firmware | Body | {device} runs Firmware {current}. Firmware {latest} adds newer Tiles. |
| Firmware | Buttons | Update, Skip |
| Firmware | Progress | Updating… {pct}% |
| Firmware | Progress hint | Keep {device} plugged in. It restarts once. |
| Firmware | Success | Updated to {latest} |
| Firmware | Failure | Couldn't update. {device} went back to {current} and still works. |
| Firmware | Failure buttons | Retry, Skip |
| Wi-Fi (radio build only) | Heading | Wi-Fi and Bluetooth |
| Wi-Fi | Body | {device} runs the radio build. Wi-Fi and Bluetooth stay off until you set them up. |
| Wi-Fi | Network field | The Device settings Wi-Fi network field, with the same strings (see Device settings, Wi-Fi) |
| Wi-Fi | Hint, with the network field | Wi-Fi is optional. You can set it up or change it in Settings later. |
| Wi-Fi | Hint, Firmware without the network field | Set up Wi-Fi in Settings, now or later. Until then, {device} works over USB as usual. |
| Done | Heading | {device} is ready |
| Done | Body | Tap a Tile on {device} to try it, or edit its Layout here. |
| Done | Buttons | Edit Layout, Done |

### New board

| Element | String |
| --- | --- |
| Detection title | New board found |
| Detection body | The board on {port} doesn't run Open TouchDeck Firmware yet. |
| Detection buttons | Install Firmware, Ignore |
| Confirm title | Install Firmware on this board? |
| Confirm warning | Installing replaces everything on the board, including any other project on it. This can't be undone. |
| Confirm buttons | Install Firmware, Cancel |
| Progress | Installing Firmware… {pct}% |
| Progress, before a percentage | Installing Firmware… |
| Declined on this PC | Declined on this PC. Nothing changed on the board. |
| Progress hint | Keep the board plugged in. This takes about a minute. |
| Success | Firmware installed |
| Success body | Next, name your Device. |
| Success button | Next |
| Failure | Couldn't install Firmware. Unplug the board, plug it back in, then retry. |
| Failure buttons | Retry, Cancel |
| Failure details | Details |
| Ignored board tooltip | Ignored. Click to install Firmware. |

### Update icon and dialog

| Element | String |
| --- | --- |
| Icon tooltip and hover label | Update available |
| Icon tooltip and hover label, update downloaded | Ready to restart |
| Dialog title | Open TouchDeck {version} |
| Dialog subtitle | You have {current}. |
| Changes heading | What's new |
| Changes, none published | No release notes. |
| Buttons | Update, Later |
| Downloading | Downloading… {pct}% |
| Downloading, size unknown | Downloading… {kb} KB |
| Downloading hint | You can keep working. |
| Ready | Ready to restart |
| Ready hint | Open TouchDeck restarts to install. Your Layouts and settings stay. |
| Ready button | Restart |
| Ready, a Device is mid-transfer | Wait for {device} to finish sending. |
| Installing | Restarting to install… |
| Failed | Couldn't download the update. Check your internet connection, then retry. |
| Failed, signature check | The download didn't match the project's signing key, so it wasn't installed. Retry later. |
| Failed button | Retry |
| Offline | You're offline. Connect to the internet, then retry. |
| Install failed | Couldn't start the installer. Retry, or download it from the releases page. |

### Device settings

A Device's page in Settings, under **Devices**. The pencil beside the Device's name in the editor renames it in place.

| Element | String |
| --- | --- |
| Name field label | Name |
| Pencil, tooltip | Rename {device} |
| Pencil, tooltip, not connected | Plug in {device} to rename it |
| Name input, accessible name | Device name |
| Name error, empty | Give the Device a name. |
| Name error, too long | That name is too long for the Device. |
| Name error, control characters | Remove tabs, line breaks and other special characters. |
| Field on older Firmware | Needs Firmware {x.y}. |
| Field no released Firmware supports | {device}'s Firmware can't change this yet. |
| Saved, takes effect after a restart | Saved. Restart {device} to apply it. |
| Orientation label | Orientation |
| Orientation options | Landscape, Portrait |
| Orientation confirm title | Restart {device} to switch to {orientation}? |
| Orientation confirm body | Each orientation keeps its own Layout. |
| Orientation confirm buttons | Restart, Cancel |
| Brightness label | Brightness |
| Screen timeout label | Screen off after |
| Screen timeout hint | {duration} without a touch. Touch to wake. |
| Photo frame label | Photo frame |
| Photo frame hint | Pictures from this PC when {device} sits idle, or a clock when Open TouchDeck isn't running. |
| Photo frame idle time label | Start after |
| Photo frame idle time options | 30 s, 1 min, 2 min, 5 min, 10 min, 15 min, 30 min, 1 hr |
| Photo frame folder label | Folder |
| Photo frame folder, none | No folder chosen |
| Photo frame folder button | Choose folder |
| Photo frame folder hint | Nothing is stored on {device}. |
| Photo frame folder hint, in a browser | Choose a folder in the Open TouchDeck app. |
| Photo frame folder error, folder gone | That folder isn't on this PC anymore. Choose another. |
| Photo frame folder dialog title | Choose a folder for the photo frame |
| Firmware label | Firmware |
| Firmware, current | Firmware {version} · Up to date |
| Firmware, behind | Firmware {version} · {latest} available |
| Firmware button | Update |
| Firmware progress | Updating… {pct}% |
| Firmware, waiting for consent | Confirm on this PC to continue. |
| Firmware done | Updated to {latest} |
| Firmware declined, or consent not answered in 5 min | Declined on this PC. {device} still runs {version}. |
| Firmware update cancelled | Update stopped. {device} still runs {version}. |
| Firmware file expired | That file expired. Choose it again, then retry. |
| Firmware file, a file dialog is already open | A file dialog is already open on this PC. Close it, then retry. |
| Restart button | Restart |
| Restart progress | Restarting… |
| Restart failed | Couldn't restart {device}. Unplug it and plug it back in. |
| Tile badge on older Firmware | Needs Firmware {x.y} |
| Tile badge, no newer Firmware offered | Needs newer Firmware |
| Tile badge tooltip | Update {device}'s Firmware to show this Tile. |
| Save failed | Couldn't save to {device}. Check that it's plugged in, then retry. |
| Not connected | Plug in {device} to change its settings. |

### Device settings, Wi-Fi

One section for the network the Device joins (radio build only, set over USB), its Pairing with this app over that network, and the Carrier. `{ssid}` is the network name the user typed.

| Element | String |
| --- | --- |
| Section label | Wi-Fi |
| Network row title | Network |
| Network, none | No network. {device} works over USB without one. |
| Network, joining | Joining {ssid}… |
| Network, connected | Connected to {ssid} at {ip} |
| Network, connected, no address yet | Connected to {ssid} |
| Network, failed | {reason} {device} keeps trying. |
| Network, reading | Reading… |
| Network, sending | Sending to {device}… |
| Network, forgetting | Forgetting… |
| Network, not connected | Plug in {device} to change its Wi-Fi network. |
| Network, over Wi-Fi | Plug in {device} with a USB cable to change its Wi-Fi network. |
| Network, older Firmware | {device}'s Firmware can't change this yet. |
| Network name label | Network name |
| Password label | Password |
| Password hint | Empty for an open network. Stored on {device} only, and never shown again. |
| Buttons | Join, Forget network |
| Network name error, empty | Type the network name. |
| Network name error, too long | That network name is too long. |
| Network name error, control characters | Remove tabs, line breaks and other special characters. |
| Network name refused by the Device | The Device refused that network name. |
| Password error, too short | A Wi-Fi password has at least 8 characters. |
| Password error, too long | A Wi-Fi password has at most 63 characters, or exactly 64 hex digits. |
| Password error, other characters | A Wi-Fi password uses only letters, digits, spaces and ASCII symbols. |
| Password refused by the Device | The Device refused that password. |
| Reason, not found | Can't find {ssid}. Check the name, and that {device} is in range. |
| Reason, wrong password | {ssid} refused the password. |
| Reason, no address | Joined {ssid}, but it gave {device} no address. |
| Reason, security | {ssid} uses security {device} doesn't support, such as WEP or WPA Enterprise. |
| Reason, refused | {ssid} turned {device} away. It may be full. |
| Reason, lost | Lost {ssid}. |
| Reason, driver | {device}'s Wi-Fi didn't start. Restart {device}. |
| Reason, other | Couldn't join {ssid}. |
| Network error, read or change failed | Couldn't {read or change} {device}'s Wi-Fi. Check that it's plugged in, then retry. |
| Pairing row title | Pairing |
| Pairing, paired | Paired at {address}. |
| Pairing, being removed (a forget on the Device went unanswered) | Being removed from {device}. Plug {device} in to finish. |
| Pairing, looking | Looking for {device} on the network… |
| Pairing, none | Not paired. Pair to use it without the cable. |
| Pairing buttons | Pair over Wi-Fi, Forget |
| Address form title | {device}'s address |
| Address form hint, found | Found {device} on the network. On {device}, choose Pair new app first. |
| Address form hint, not found | Type the address {device} shows under Wi-Fi. On {device}, choose Pair new app first. |
| Pairing notice (net-v2: physical access to the Device is access to its Pairings) | Whoever can get to {device} can use every Pairing on it. If {device} is lost or passed on, forget its paired apps. |
| Address placeholder | IP address or name |
| Address form buttons | Cancel, Start pairing |
| Code form title | Enter the code on {device} |
| Code form hint | {device} at {address} shows {n} digits. |
| Code tries left | 1 try left, {n} tries left |
| Code form buttons | Cancel, Pair |
| Code error, wrong length | Enter the {n} digits {device} shows. |
| Pairing error, nothing answered | Nothing answered at that address. Check that {device} is on the same network, and the address. |
| Pairing error, another Device | Another Device answered at that address, not {device}. |
| Pairing error, not pairing | {device} isn't pairing. On {device}, open Settings and choose Pair new app, then try again. |
| Pairing error, wrong code | Wrong code. {n} tries left. |
| Pairing error, locked | Too many wrong codes. {device} stopped pairing for a while. Wait, then start again. |
| Pairing error, window ended | Pairing ended on {device}. Start again. |
| Pairing error, four apps | Four apps are paired. Forget one first: on {device}, use the paired apps screen, or plug it in with a USB cable. |
| Pairing error, already paired | {device} still has a Pairing with this app. Forget it on {device} (paired apps screen), then pair again. |
| Pairing error, old Firmware | {device}'s Firmware can't pair over Wi-Fi with this app. Update its Firmware over USB, then try again. |
| Pairing error, busy | {device} is busy with another app. Close it, then try again. |
| Pairing error, attempt gone | That pairing ended. Start again. |
| Carrier, USB | Connected over USB |
| Carrier, Wi-Fi | Connected over Wi-Fi at {address} |

### Device settings, Bluetooth keyboard

Shown only for a Device whose Firmware has the Bluetooth keyboard (the radio build). It changes only over USB. A Keyboard host is a computer or phone it types on; the copy says "computer or phone" and keeps "pair" for the Wi-Fi Pairing.

| Element | String |
| --- | --- |
| Section label and switch name | Bluetooth keyboard |
| Status, off | Off. Turn it on to type on a computer or phone with {device}. |
| Status, turning on | Turning on… |
| Status, turning off | Turning off… |
| Status, starting | Starting… |
| Status, didn't start (only while the switch is on) | Bluetooth didn't start on {device}. Restart {device} to try again. |
| Status, no Keyboard hosts | On. Add a computer or phone to type with {device}. |
| Status, one connected | On. Connected to a computer or phone. |
| Status, none connected | On. Waiting for a computer or phone to connect. |
| Status, adding | Waiting for a computer or phone… |
| Status, code on the Device | {device} shows a 6-digit code. Type it on the computer or phone. |
| Status, reading | Reading… |
| Not connected | Plug in {device} to change its Bluetooth keyboard. |
| Over Wi-Fi | Plug in {device} with a USB cable to change its Bluetooth keyboard. |
| Older Firmware | {device}'s Firmware can't change this yet. |
| Add button | Add a computer or phone |
| Adding box title | Adding a computer or phone |
| Adding countdown | {m:ss}, read as "{m:ss} left" |
| Adding hint | Open Bluetooth on the computer or phone and choose {device}. {device} shows a 6-digit code; type it on the computer or phone. |
| Adding button | Stop adding |
| Full | {device} keeps {n} computers and phones. Forget one to add another. |
| Keyboard host list name (screen readers) | Keyboard hosts |
| Keyboard host row | {address}, then Connected or Not connected |
| Forget button | Forget, read as "Forget {address}"; in progress Forgetting… |
| Forget all button | Forget all |
| Forget all confirm title | Forget every computer and phone? |
| Forget all confirm body | Each one needs a new code from {device} to be added again. |
| Forget all confirm buttons | Cancel, Forget all; in progress Forgetting… |
| Error, off | Turn on the Bluetooth keyboard first. |
| Error, starting | Bluetooth is still starting on {device}. Try again in a moment. |
| Error, didn't start | Bluetooth didn't start on {device}. Restart {device} to try again. |
| Error, full | {device} keeps {n} computers and phones. Forget one to add another. |
| Error, couldn't save | {device} couldn't save that. Try again. |
| Error, over Wi-Fi | Plug in {device} with a USB cable to change its Bluetooth keyboard. |
| Error, no answer (reading again by itself, waiting longer each time) | Couldn't reach {device}. Check that it's plugged in, then retry. |

### Device panel, Paired apps

Shown only for a Device whose Firmware keeps Pairings (the radio build). The list and the forgets work only over USB. The apps are named as they paired; no key ever shows.

| Element | String |
| --- | --- |
| Section label | Paired apps |
| Count | {n} of {max} |
| Status, reading | Reading… |
| Status, none | No apps are paired. Pair one over Wi-Fi to use {device} without the cable. |
| Status, full | Four apps are paired. Forget one to pair another. |
| Not connected | Plug in {device} to see its paired apps. |
| Over Wi-Fi | Plug in {device} with a USB cable to see its paired apps. |
| Older Firmware | {device}'s Firmware can't change this yet. |
| App row | {app name}, with This app under this app's own |
| Forget button | Forget, read as "Forget {app name}"; in progress Forgetting… |
| Forget all button | Forget all |
| Forget all confirm title | Forget every paired app? |
| Forget all confirm body | None of them can reach {device} over Wi-Fi until it pairs again with a new code. |
| Forget all confirm buttons | Cancel, Forget all; in progress Forgetting… |
| Error, couldn't erase | {device} couldn't erase that. Try again. |
| Error, over Wi-Fi | Plug in {device} with a USB cable to change its paired apps. |
| Error, no answer to a forget | {device} didn't answer, so the app may still be paired. Plug {device} in again to finish. |
| Error, no answer | Couldn't reach {device}. Check that it's plugged in, then retry. |

### On the Device, Settings (radio build)

The Firmware draws these on the Device's own screen. A gear at the right end of the title strip opens Settings; a Layout without a title strip shows no gear. The Firmware's font has no `…`, so in-progress text ends in three dots there.

| Element | String |
| --- | --- |
| Panel title and its close button | Settings, Done |
| Section | Paired apps, with "{n} of 4" |
| Empty list | No apps are paired. Choose Pair new app, then type the code this Device shows into the app. |
| Row button | Forget |
| Buttons | Pair new app, Forget all |
| Saving | Saving... |
| Full | Four apps are paired. Forget one first. |
| Locked | Too many wrong codes. Pair again in {m:ss}. |
| Forget one confirm | Forget {app name}? It can't reach this Device over Wi-Fi until you pair it again. Cancel, Forget |
| Forget all confirm | Forget every paired app? None of them can reach this Device over Wi-Fi until you pair it again. Cancel, Forget all |

### Integrations

| Element | String |
| --- | --- |
| Page title | Integrations |
| Page subtitle | Connect apps and services, then add their Preset tiles to any Page. |
| Status, working | Connected |
| Status, turned off | Off |
| Status, first check | Checking… |
| Status, failing | Can't connect |
| Status, no credentials | Not set up |
| Check button, in progress | Check now, Checking… |
| Check sent (new values arrive when it ends) | Check started |
| Card button, not set up | Set up |
| Secret field, saved | Saved · ends in {last4} |
| Secret field, empty | Not set |
| Secret field buttons | Save, Forget |
| Secret field hint | Stored in {keychain}, never in a file. `{keychain}` is "Windows Credential Manager" on Windows and "your system keychain (GNOME Keyring or KWallet)" on Linux |
| Forget confirm | Forget the {field}? {integration} stops until you add it again. |
| Preset tile button | Add to Layout |
| Preset tile added | Added to {device}'s Layout |
| Preset tile, Device too old | Needs Firmware {x.y} |
| Can't connect, details link | Details |
| Custom Integration, command approval | Let this Integration run a command? |

### Settings

| Element | String |
| --- | --- |
| Page title, nav | Settings |
| Category labels | General, Devices, App |
| Categories | General, {device} (one per Device), Paired browsers, Troubleshooting, Errors, About |
| Category, no Device | No Device |
| Errors, lede | What went wrong since Open TouchDeck opened, newest first. |
| Errors, empty | No errors. |
| Errors, clear button | Clear |
| Errors, copy button label | Copy |
| Error card, where | Layout · {device} |
| Error card button | Details |
| Start at sign-in toggle | Start at sign-in |
| Start at sign-in hint | Runs in the tray when you sign in to Windows, so your Devices work right away. |
| Appearance label | Appearance |
| Appearance options | Match Windows, Light, Dark |
| Paired browsers heading | Paired browsers |
| Paired browsers, empty | No browsers paired. Choose Open in browser in the tray to pair one. |
| Troubleshooting entry | Troubleshooting |
| Troubleshooting, update from a file heading | Update from a file |
| Troubleshooting, update from a file hint | A .tdfw file from an Open TouchDeck release. |
| Troubleshooting, update from a file button | Choose file |
| About entry | About |
| About, version | Open TouchDeck {version} |
| About, build | Build {sha} |
| About button | Check for updates |
| About, checking | Checking… |
| About, current | Up to date |

### Tray menu

| Element | String |
| --- | --- |
| Tray tooltip | Open TouchDeck |
| Tray tooltip, Device needs attention | Open TouchDeck · {device} not connected |
| Menu item | Open |
| Menu item | Open in browser |
| Menu item | Quit |

### Browser pairing

1.x only: 2.0 has no browser version. The **Open in browser** tray item and the **Paired browsers** Settings category above are 1.x only too.

| Element | String |
| --- | --- |
| Prompt title | Pair this browser |
| Prompt body | Enter the code Open TouchDeck shows on this PC. |
| Code field label | Code |
| Button | Pair |
| Progress | Pairing… |
| Paired | Paired. This browser can now use Open TouchDeck. |
| Wrong code | That code didn't match. Check it, then retry. |
| Expired code | That code expired. Choose Open in browser in the tray for a new one. |
| Companion unreachable | Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry. |
| Paired browser row | {browser} · paired {date} |
| Forget button | Forget |
| Forget confirm title | Forget this browser? |
| Forget confirm body | It needs a new code to use Open TouchDeck again. |
| Forget confirm buttons | Forget, Cancel |

### Icon picker

Opens from the Icon field in the Tile inspector.

| Element | String |
| --- | --- |
| Icon field button | Change |
| Icon field, a Tile without an icon | No icon |
| Icon field, an Asset no recent pick names | Custom icon |
| Picker title, for screen readers | Choose icon |
| Search placeholder | Search icons, like Spotify or mail |
| Search label | Search icons |
| Hint, before any search or recent pick | Search Simple Icons and selfh.st logos, or pick a built-in icon. |
| Section titles | Recent, Logos, Built-in |
| First built-in cell | No icon |
| Logo cell tooltip | {name} · {source} |
| Searching | Searching… |
| No results | No icons match “{query}”. Try a shorter word. |
| Companion unreachable | Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry. |
| Search or built-ins failed | Icons didn't load. Restart Open TouchDeck, then retry. |
| Getting a logo | Getting {name}… |
| Logo failed | Couldn't get {name}. Check the internet connection, then retry. |
| Upload button | Upload image |
| Uploading | Uploading “{file}”… |
| Upload failed | Couldn't upload “{file}”. Choose a PNG, JPEG or SVG image up to 16 MB. |
| Icon pack cell tooltip | {name} · {pack name} |
| Icon packs button | Get more icons |
| App icon button, launch actions only | Use app icon |
| Getting the app icon | Getting icon… |
| App icon failed | Couldn't get that app's icon. Check the app's path, or choose an icon. |
| Recent app icon name | {program file name}, or App icon |
| Too many icons rendering at once | Other icons are still being prepared. Wait a moment, then retry. |
| Failure buttons | Retry, Details |
| Credits | Icons: Simple Icons (CC0) · selfh.st/icons (CC BY 4.0, modified)[ · {pack name} by {author} ({license}, modified), for each installed Icon pack that asks for credit][ · Brandfetch (your keys), while a Brandfetch client ID is saved] · Logos belong to their owners |

### Icon packs

Opens from Settings, and from Get more icons in the icon picker.

| Element | String |
| --- | --- |
| Settings entry | Icon packs |
| Settings entry subtitle | Download Icon packs to search offline, or import your own. |
| View title | Icon packs |
| Lede | Download Icon packs to search offline in the icon picker, or import your own. Nothing is installed until you choose. |
| Curated heading | Packs |
| Pack facts | {count} icons · {size} · {license}[ · Credit required][ · Logos] |
| Loading | Loading… |
| List failed | Icon packs didn't load. Restart Open TouchDeck, then retry. |
| Download button | Download |
| Install stages | Downloading…, Checking…, Unpacking…, Finishing… |
| Remove button | Remove |
| Removing | Removing… |
| Checksum mismatch | {pack} didn't match its checksum, so it wasn't installed. Retry later. |
| Download failed (`icon_pack.download_failed`) | Couldn't download {pack}. Check the internet connection, then retry. |
| Package unusable (`icon_pack.invalid`) | {pack} couldn't be unpacked, so it wasn't installed. Update Open TouchDeck, then retry. |
| Couldn't write the pack (`internal`) | Couldn't save {pack} on this PC. Check that the disk has free space, then retry. |
| Any other install failure | Couldn't install {pack}. Restart Open TouchDeck, then retry. |
| Under any failure | Details, holding the Companion's own message |
| Too many installs | Other packs are still downloading. Wait for them, then retry. |
| Install cancelled | {pack} wasn't installed. |
| Retry button | Retry |
| Remove failed | Couldn't remove {pack}. Restart Open TouchDeck, then retry. |
| Remove while installing | {pack} is still installing. Wait for it, then retry. |
| Imported heading | Your packs |
| Import button | Import a pack file |
| Importing | Importing… |
| No imported packs | Import an Iconify JSON file to search its icons here. Its license is yours to check. |
| Imported license | Custom (you provided it) |
| Import clash | A pack with the same prefix as “{file}” is already installed or offered above. |
| Import too large | “{file}” is larger than 32 MB. |
| Import invalid | “{file}” isn't an Iconify JSON pack. Choose an Iconify JSON file, such as icons.json. |
| Too many imports | You can import up to 20 packs. Remove one, then import “{file}” again. |
| Footnote | Downloads come from the npm registry and must match a pinned checksum. Logos belong to their owners. |

## Tooltip names for the nav icons

| Icon | Tooltip |
| --- | --- |
| Home | Home |
| Layout | Layout |
| Integrations | Integrations |
| Settings | Settings |
| Update, shown only when one exists | Update available |
| Device name, in the header | Device settings |

## Old-to-new table

Every user-visible string in today's app. Locations are relative to `app/`. "Dropped" gives the reason. Placeholders in braces are values the app fills in.

### Shell and navigation

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `index.html:6` | TouchDeck | Open TouchDeck | Page title (ADR 0003) |
| `app/electron/electron-builder.json` (`productName`) | TouchDeck | Open TouchDeck | Product name for the installer and taskbar |
| `app/electron/src/main.ts` (window `title`) | TouchDeck | Open TouchDeck | Window title |
| `src-tauri/src/lib.rs:159` (removed) | TouchDeck {BUILD_SHA} | Open TouchDeck | Build hash moves to Settings, About |
| `src/App.svelte:13` | TouchDeck | Open TouchDeck | |
| `src/App.svelte:18` | Buttons | Layout | Bottom-left icon, tooltip "Layout" |
| `src/App.svelte:19` | AI usage | Integrations | Bottom-left icon. AI usage becomes the CLIProxyAPI Integration |
| `src/App.svelte:20` | Settings | Settings | Bottom-left icon, opens the Settings pop-up |
| `src/App.svelte:51` | TouchDeck | Open TouchDeck | |
| `src/App.svelte:53` | Main | Main | Nav aria-label |

### Command approval

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/actions/Approval.svelte:48` | Let this button run a command? | Let this Tile run a command? | |
| `src/lib/actions/Approval.svelte:49` | The button {tile} on page {page} wants to run this on your computer. Only allow it if you set it up. | {tile} on the {page} Page wants to run this on this PC. Allow it only if you set it up. | |
| `src/lib/actions/Approval.svelte:51` | Program | Program | |
| `src/lib/actions/Approval.svelte:53` | With | Arguments | |
| `src/lib/actions/Approval.svelte:59` | nothing else | None | |
| `src/lib/actions/Approval.svelte:63` | In folder | Folder | |
| `src/lib/actions/Approval.svelte:64` | TouchDeck's folder | Open TouchDeck's folder | |
| `src/lib/actions/Approval.svelte:65` | System shell | System shell | |
| `src/lib/actions/Approval.svelte:66` | Yes, it runs through the system shell / No | Yes / No | The label already says shell |
| `src/lib/actions/Approval.svelte:69` | {n} more button is / buttons are waiting after this one. | {n} more waiting | |
| `src/lib/actions/Approval.svelte:72` | TouchDeck couldn't pass on your answer. Press the button on the deck again. ({problem}) | Couldn't send your answer. Tap the Tile again. | Raw reason under Details |
| `src/lib/actions/Approval.svelte:75` | Don't run | Don't run | |
| `src/lib/actions/Approval.svelte:76` | Runs this exact command from now on without asking / Always allow | Run this exact command from now on without asking / Always allow | Tooltip, then button |
| `src/lib/actions/Approval.svelte:77` | Run once | Run once | |

### Action log (was Button activity)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/actions/ButtonActivity.svelte:48` | started (process {pid}) | Started (process {pid}) | Added by hand: one lowercase word |
| `src/lib/actions/ButtonActivity.svelte:50` | finished / finished with code {code} | Finished / Exited with code {code} | |
| `src/lib/actions/ButtonActivity.svelte:52` | not run, you said no | Denied | |
| `src/lib/actions/ButtonActivity.svelte:54` | didn't work | Failed | Reason under Details |
| `src/lib/actions/ButtonActivity.svelte:56` | ignored, it isn't in the layout TouchDeck knows | Ignored, not in {device}'s Layout | |
| `src/lib/actions/ButtonActivity.svelte:77` | Buttons come from “{name}” | Using “{layout}” from {device} | |
| `src/lib/actions/ButtonActivity.svelte:78` | ({read from your deck or the file name}), {n} page(s), {n} button(s). | : {n} Pages, {n} Tiles | The source is always the Device |
| `src/lib/actions/ButtonActivity.svelte:80` | TouchDeck reads your deck's buttons when it connects. | Dropped | The Companion reads each Device's Layout on its own |
| `src/lib/actions/ButtonActivity.svelte:84` | Use a layout file instead… | Dropped | Actions always run from the Device's own Layout |
| `src/lib/actions/ButtonActivity.svelte:89` | That file couldn't be used: {problem} | Dropped | Goes with the layout file picker |
| `src/lib/actions/ButtonActivity.svelte:100` | No presses yet. Press a button on your deck that opens an app or runs a command. | No actions yet. Tap a Tile that opens an app or runs a command. | |

### Key test (was Test your keys)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/actions/KeyMonitor.svelte:73` | Click here, then press a button on your deck | Click here, then tap a Tile on {device} | |
| `src/lib/actions/KeyMonitor.svelte:74` | Key tester | Key test | aria-label |
| `src/lib/actions/KeyMonitor.svelte:78` | Clear | Clear | |
| `src/lib/actions/KeyMonitor.svelte:82` | What the deck sent | Sent by {device} | |
| `src/lib/actions/KeyMonitor.svelte:86` | (missed {from} to {to}, so every key was let go) / ({typed, duplicate, off or unknown}) | (missed {from} to {to}, released all keys) / (duplicate, off or unknown) | Outcome words stay, they're diagnostics |
| `src/lib/actions/KeyMonitor.svelte:92` | What this computer typed | Typed on this PC | |
| `src/lib/actions/KeyMonitor.svelte:95` | {down or up} {key} | {down or up} {key} | Diagnostic, unchanged |

### Connection messages

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/connection.ts:96` | TouchDeck, software {version} | Dropped | The Device card shows its name, the Device panel its Firmware |
| `src/lib/connection.ts:98` | Didn't answer yet. It may still be starting up. | Connecting… | Card status |
| `src/lib/connection.ts:100` | Another program is using it. Close that program, then look again. | In use by another program | Card status, with the hint "Close the other program. {device} connects on its own." |
| `src/lib/connection.ts:102` | Not a TouchDeck | Dropped | Espressif boards without Firmware are New boards, other ports are ignored |
| `src/lib/connection.ts:104` | Can't be opened right now | Can't open {port}. Unplug the Device and plug it back in. | Troubleshooting |
| `src/lib/connection.ts:112` | Another program is using the deck. Close it, then connect again. | Another program is using {device}. Close it, and {device} reconnects on its own. | |
| `src/lib/connection.ts:115` | The deck didn't answer. Unplug it, plug it back in, then connect again. | {device} didn't answer. Unplug it and plug it back in. | |
| `src/lib/connection.ts:117` | The connection to your deck dropped. Check the USB cable, then connect again. | Lost {device}. Check the USB cable. It reconnects on its own. | |

### Device preview

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/deck/DeckScreen.svelte:406` | {page} buttons | {page} Tiles | aria-label, added by hand |
| `src/lib/deck/DeckScreen.svelte:422` | Add a button in row {r}, column {c} | Add Tile at row {r}, column {c} | aria-label |
| `src/lib/deck/scene.ts:252` | codex alex | codex alex | Sample row on the Usage page preview |
| `src/lib/deck/scene.ts:253` | claude sam | claude sam | Sample row |
| `src/lib/deck/scene.ts:254` | gemini-cli jo | gemini-cli jo | Sample row |
| `src/lib/deck.ts:50` | TouchDeck {version} | Dropped | `describe()` has no caller today |
| `src/lib/deck.ts:52` | No answer to hello | Dropped | No caller |
| `src/lib/deck.ts:54` | In use by another program | Dropped | No caller. Home uses the same words as a card status |

### Tile action form

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/ActionForm.svelte:20` | Play or pause | Play or pause | |
| `src/lib/editor/ActionForm.svelte:21` | Next track | Next track | |
| `src/lib/editor/ActionForm.svelte:22` | Previous track | Previous track | |
| `src/lib/editor/ActionForm.svelte:23` | Stop | Stop | Added by hand |
| `src/lib/editor/ActionForm.svelte:24` | Mute | Mute | Added by hand |
| `src/lib/editor/ActionForm.svelte:25` | Volume up | Volume up | |
| `src/lib/editor/ActionForm.svelte:26` | Volume down | Volume down | |
| `src/lib/editor/ActionForm.svelte:29` | Ctrl, Shift, Alt, Windows key | Ctrl, Shift, Alt, Windows key | |
| `src/lib/editor/ActionForm.svelte:34` | On this computer, like a keyboard | Keyboard | Option group |
| `src/lib/editor/ActionForm.svelte:36` | Press a key or shortcut | Press a shortcut | |
| `src/lib/editor/ActionForm.svelte:37` | Control music and volume | Media key | |
| `src/lib/editor/ActionForm.svelte:38` | Hold Ctrl, Shift, Alt, or Windows | Hold Ctrl, Shift, Alt or Windows | |
| `src/lib/editor/ActionForm.svelte:42` | On this computer, through TouchDeck | This PC | Option group |
| `src/lib/editor/ActionForm.svelte:44` | Open an app | Open an app | |
| `src/lib/editor/ActionForm.svelte:45` | Open a file, folder, or website | Open a file, folder or website | |
| `src/lib/editor/ActionForm.svelte:46` | Run a command | Run a command | |
| `src/lib/editor/ActionForm.svelte:50` | On the deck | Device | Option group |
| `src/lib/editor/ActionForm.svelte:52` | Go to another page | Go to a Page | |
| `src/lib/editor/ActionForm.svelte:53` | Show AI usage | Dropped | Usage Tiles come from CLIProxyAPI's Preset tiles, through Add to Layout |
| `src/lib/editor/ActionForm.svelte:54` | Nothing | Nothing | Added by hand |
| `src/lib/editor/ActionForm.svelte:61` | 5-hour limit used | 5-hour limit used | Preset tile value |
| `src/lib/editor/ActionForm.svelte:62` | 7-day limit used | 7-day limit used | |
| `src/lib/editor/ActionForm.svelte:63` | Time until the 5-hour limit resets | 5-hour reset in | |
| `src/lib/editor/ActionForm.svelte:64` | Time until the 7-day limit resets | 7-day reset in | |
| `src/lib/editor/ActionForm.svelte:65` | Plan name | Plan | |
| `src/lib/editor/ActionForm.svelte:66` | Account status | Account status | |
| `src/lib/editor/ActionForm.svelte:67` | Requests that worked | Requests OK | |
| `src/lib/editor/ActionForm.svelte:68` | Requests that failed | Requests failed | |
| `src/lib/editor/ActionForm.svelte:69` | Requests in the last 10 minutes | Requests in 10 min | |
| `src/lib/editor/ActionForm.svelte:72` | Your CLIProxyAPI hub | CLIProxyAPI | |
| `src/lib/editor/ActionForm.svelte:186` | What the button does | Action | Visually hidden label |
| `src/lib/editor/ActionForm.svelte:200` | Key | Key | |
| `src/lib/editor/ActionForm.svelte:203` | Key code {n} | Key code {n} | |
| `src/lib/editor/ActionForm.svelte:211` | Hold down with it | Hold with it | |
| `src/lib/editor/ActionForm.svelte:226` | Presses {shortcut} on this computer. | Presses {shortcut} on this PC. | |
| `src/lib/editor/ActionForm.svelte:229` | Key | Key | |
| `src/lib/editor/ActionForm.svelte:236` | Tap it once and it's held for your next key. Tap it twice quickly and it stays on until you tap it again. | Tap once to hold it for the next key. Double-tap to lock it until you tap again. | |
| `src/lib/editor/ActionForm.svelte:239` | Control | Key | |
| `src/lib/editor/ActionForm.svelte:242` | Media code {n} | Media code {n} | |
| `src/lib/editor/ActionForm.svelte:251` | Page | Page | |
| `src/lib/editor/ActionForm.svelte:260` | App | App | |
| `src/lib/editor/ActionForm.svelte:261` | notepad.exe, or the full path to an app | notepad.exe or a full path | Placeholder |
| `src/lib/editor/ActionForm.svelte:264` | More options | More options | |
| `src/lib/editor/ActionForm.svelte:267` | Start it with, one per line | Arguments, one per line | Same label as Run a command |
| `src/lib/editor/ActionForm.svelte:271` | Start it in this folder | Start in folder | |
| `src/lib/editor/ActionForm.svelte:278` | File, folder, or web address | File, folder or web address | |
| `src/lib/editor/ActionForm.svelte:279` | https://example.com | https://example.com | Placeholder, added by hand |
| `src/lib/editor/ActionForm.svelte:281` | Opens it the way double-clicking it would. | Opens it as a double-click would. | |
| `src/lib/editor/ActionForm.svelte:284` | Program | Program | |
| `src/lib/editor/ActionForm.svelte:288` | Arguments, one per line | Arguments, one per line | |
| `src/lib/editor/ActionForm.svelte:292` | Run it in this folder | Run in folder | |
| `src/lib/editor/ActionForm.svelte:293` | TouchDeck's folder | Open TouchDeck's folder | Placeholder |
| `src/lib/editor/ActionForm.svelte:295` | Run it through the system shell | Run through the system shell | |
| `src/lib/editor/ActionForm.svelte:296` | TouchDeck shows you the exact command and asks before it runs. | Open TouchDeck shows the exact command and asks before it runs. | |
| `src/lib/editor/ActionForm.svelte:299` | From | Integration | Preset tile settings |
| `src/lib/editor/ActionForm.svelte:307` | Show | Show | |
| `src/lib/editor/ActionForm.svelte:310` | {field} (not recognised) | {field} (unknown) | |
| `src/lib/editor/ActionForm.svelte:318` | Account | Account | |
| `src/lib/editor/ActionForm.svelte:321` | All accounts | All accounts | |
| `src/lib/editor/ActionForm.svelte:322` | {account} (not seen lately) | {account} (not seen lately) | |
| `src/lib/editor/ActionForm.svelte:326` | All accounts | All accounts | Placeholder |
| `src/lib/editor/ActionForm.svelte:329` | The deck shows the number with a bar, and updates it while TouchDeck runs. Set up the hub under AI usage. | Shows the number with a bar. Set up CLIProxyAPI under Integrations. | |
| `src/lib/editor/ActionForm.svelte:331` | The button shows on the deck but does nothing. Handy as a label. | The Tile shows but does nothing. Use it as a label. | |

### Icon picker

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/IconPicker.svelte:31` | The picture gallery didn't load. Restart TouchDeck to try again. ({error}) | Icons didn't load. Restart Open TouchDeck, then retry. | Raw reason under Details |
| `src/lib/editor/IconPicker.svelte:61` | Couldn't take that app's icon. Check the app's path, or choose a picture instead. ({error}) | Couldn't get that app's icon. Check the app's path, or choose an icon. | Raw reason under Details |
| `src/lib/editor/IconPicker.svelte:75` | Hide pictures / Choose a picture | Hide icons / Choose icon | |
| `src/lib/editor/IconPicker.svelte:77` | Getting icon… / Use the app's icon | Getting icon… / Use app icon | |
| `src/lib/editor/IconPicker.svelte:80` | Suggested for what this button does | Suggested for this action | Tooltip |
| `src/lib/editor/IconPicker.svelte:82` | Suggested: {label} | Suggested: {label} | |
| `src/lib/editor/IconPicker.svelte:88` | Search pictures, like music or mail / Search pictures | Search icons, like Spotify or mail / Search icons | Placeholder, then aria-label |
| `src/lib/editor/IconPicker.svelte:104` | No picture matches “{query}”. Try a shorter word. | No icons match “{query}”. Try a shorter word. | |
| `src/lib/editor/IconPicker.svelte:107` | Frees space on the deck / Clear unused pictures from the deck | Frees space on {device} / Remove unused Assets | Tooltip, then button |

### Page and Layout details

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/Inspector.svelte:69` | Page {n} of {total} | Page {n} of {total} | |
| `src/lib/editor/Inspector.svelte:70` | This page / AI usage page | Page / Usage page | |
| `src/lib/editor/Inspector.svelte:73` | Page name | Name | The heading already says Page |
| `src/lib/editor/Inspector.svelte:75` | Shown at the top left of the deck. Up to 24 characters. | Shown at the top left of {device}. Up to 24 characters. | |
| `src/lib/editor/Inspector.svelte:80` | Grid | Grid | |
| `src/lib/editor/Inspector.svelte:82` | Rows | Rows | aria-label |
| `src/lib/editor/Inspector.svelte:83` | {n} row / rows | {n} row / rows | Added by hand |
| `src/lib/editor/Inspector.svelte:85` | × | × | Added by hand |
| `src/lib/editor/Inspector.svelte:86` | Columns | Columns | aria-label |
| `src/lib/editor/Inspector.svelte:87` | {n} column / columns | {n} column / columns | Added by hand |
| `src/lib/editor/Inspector.svelte:90` | More rows and columns make smaller buttons. Empty squares are left blank on the deck. | More rows and columns make smaller Tiles. Empty squares stay blank. | |
| `src/lib/editor/Inspector.svelte:93` | This page lists each AI account with how much of its limit is used. Set it up under AI usage. The rows on the deck are examples until then. | Lists each CLIProxyAPI account and how much of its limit is used. Shows examples until CLIProxyAPI is set up under Integrations. | |
| `src/lib/editor/Inspector.svelte:100` | A layout needs at least one page | A Layout needs at least one Page | Tooltip |
| `src/lib/editor/Inspector.svelte:101` | Click again to remove “{title}” / Remove this page | Click again to remove “{page}” / Remove Page | |
| `src/lib/editor/Inspector.svelte:107` | Whole layout | Dropped | The heading below says Layout |
| `src/lib/editor/Inspector.svelte:108` | All pages | Layout | Heading |
| `src/lib/editor/Inspector.svelte:111` | Layout name | Name | |
| `src/lib/editor/Inspector.svelte:117` | Use the whole screen | Full screen | |
| `src/lib/editor/Inspector.svelte:118` | Hide the page name and the page dots so the buttons fill the screen. You can still swipe between pages. Needs the latest deck software. | Hide the Page name and dots so Tiles fill the screen. Swiping between Pages still works. Needs Firmware {x.y}. | |
| `src/lib/editor/Inspector.svelte:125` | Use the whole screen | Full screen | aria-label |
| `src/lib/editor/Inspector.svelte:132` | The deck can't show {chars}. It draws an empty box instead. | {device} can't show {chars} and draws an empty box instead. | |

### Layout menu

The menu button becomes an icon button.

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/LayoutMenu.svelte:46` | Layout | More | Icon button tooltip. "Layout" is now the nav item |
| `src/lib/editor/LayoutMenu.svelte:50` | Layout | Layout options | Menu aria-label |
| `src/lib/editor/LayoutMenu.svelte:52` | Working… / Load from deck | Loading… / Load from {device} | |
| `src/lib/editor/LayoutMenu.svelte:53` | Replace these buttons with the ones on your deck / Connect your deck first | Replace this Layout with the one on {device} / {device} isn't connected | |
| `src/lib/editor/LayoutMenu.svelte:56` | Open a file… | Import | |
| `src/lib/editor/LayoutMenu.svelte:57` | A layout you saved before | A Layout file you exported | |
| `src/lib/editor/LayoutMenu.svelte:60` | Save to a file… | Export | |
| `src/lib/editor/LayoutMenu.svelte:61` | Keep a copy or share it | Save a copy to share or keep | |
| `src/lib/editor/LayoutMenu.svelte:65` | Start with examples | Starter Layout | Same name as in onboarding |
| `src/lib/editor/LayoutMenu.svelte:66` | Media controls and shortcuts | Media controls and shortcuts | |
| `src/lib/editor/LayoutMenu.svelte:69` | Start blank | Blank Layout | |
| `src/lib/editor/LayoutMenu.svelte:70` | One empty page | One empty Page | |

### Layout status messages

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/layoutStore.ts:78` | {n} page(s), {n} button(s) | {n} Pages, {n} Tiles | Added by hand |
| `src/lib/editor/layoutStore.ts:147` | Started a blank layout with one empty page. | Started a Blank Layout. | |
| `src/lib/editor/layoutStore.ts:151` | Started with media controls and everyday shortcuts. Change anything you like. | Started the Starter Layout. | |
| `src/lib/editor/layoutStore.ts:162` | Opened “{name}”: {n} pages, {n} buttons. | Imported “{layout}”: {n} Pages, {n} Tiles. | |
| `src/lib/editor/layoutStore.ts:193` | Page {n} | Page {n} | Default name for a new Page |
| `src/lib/editor/layoutStore.ts:203` | Sending “{name}” to your deck… | Sending… | |
| `src/lib/editor/layoutStore.ts:210` | , {n} icon(s) not in the cache | , {n} icons not cached | Details only |
| `src/lib/editor/layoutStore.ts:211` | Deck shows {name}: {n} pages, {n} tiles ({bytes} bytes in {ms} ms), {n} icon(s) sent{missing} | {device} shows {layout}: {n} Pages, {n} Tiles ({bytes} bytes in {ms} ms), {n} icons sent{missing} | Details only |
| `src/lib/editor/layoutStore.ts:212` | Sent to your deck. It now shows “{name}”: {n} pages, {n} buttons. | Sent to {device} | |
| `src/lib/editor/layoutStore.ts:213` | that button shows only its name / those buttons show only their names | Dropped | Folded into the line below |
| `src/lib/editor/layoutStore.ts:216` | {shows} {n} icon(s) couldn't be found on this computer, so {lost}. | Sent to {device}, but {n} icons are missing on this PC, so those Tiles show only their names. | |
| `src/lib/editor/layoutStore.ts:221` | Couldn't send to your deck. Check that it's plugged in and connected, then try again. | Couldn't send to {device}. Check that it's plugged in, then retry. | |
| `src/lib/editor/layoutStore.ts:233` | Clearing icons your deck no longer uses… | Removing unused Assets… | |
| `src/lib/editor/layoutStore.ts:236` | Removed {n} unused icon(s) from your deck. | Removed {n} unused Assets from {device} | |
| `src/lib/editor/layoutStore.ts:239` | Couldn't clear unused icons. Check that your deck is connected, then try again. | Couldn't remove unused Assets. Check that {device} is plugged in, then retry. | |
| `src/lib/editor/layoutStore.ts:247` | Loading the {orientation} layout from your deck… | Loading… | |
| `src/lib/editor/layoutStore.ts:253` | Your deck's layout has a problem this app can't open. | {device}'s Layout has a problem Open TouchDeck can't read. Nothing changed. | |
| `src/lib/editor/layoutStore.ts:259` | Your deck has “{name}”: {n} pages, {n} buttons. | {device} has “{layout}”: {n} Pages, {n} Tiles. | |
| `src/lib/editor/layoutStore.ts:262` | Couldn't load from your deck. Check that it's plugged in and connected, then try again. | Couldn't load from {device}. Check that it's plugged in, then retry. | |
| `src/lib/editor/layoutStore.ts:270` | Loaded “{name}” from your deck: {n} pages, {n} buttons. | Loaded “{layout}” from {device} | |
| `src/lib/editor/layoutStore.ts:274` | Kept your layout. Nothing changed. | Kept your Layout | |

### Layout rule messages

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/messages.ts:17` | Choose the app to open. | Choose an app. | |
| `src/lib/editor/messages.ts:18` | Enter the file, folder, or web address to open. | Enter a file, folder or web address. | |
| `src/lib/editor/messages.ts:19` | Enter the program to run. | Enter a program. | |
| `src/lib/editor/messages.ts:20` | Choose where the usage numbers come from. | Choose an Integration. | |
| `src/lib/editor/messages.ts:21` | Choose which number to show. | Choose a value to show. | |
| `src/lib/editor/messages.ts:29` | That spot is taken by “{label}”. Pick an empty spot. | That spot is taken by “{tile}”. Pick an empty one. | Added by hand: nested template |
| `src/lib/editor/messages.ts:32` | That doesn't fit. This page has {n} rows and {n} columns. | That doesn't fit. This Page has {n} rows and {n} columns. | |
| `src/lib/editor/messages.ts:34` | This page is full. Make its grid bigger or add another page. | This Page is full. Make its grid bigger or add a Page. | |
| `src/lib/editor/messages.ts:35` | A page can hold up to {n} buttons. | A Page holds up to {n} Tiles. | |
| `src/lib/editor/messages.ts:36` | A layout can have up to {n} pages. | A Layout holds up to {n} Pages. | |
| `src/lib/editor/messages.ts:37` | A layout needs at least one page. | A Layout needs at least one Page. | |
| `src/lib/editor/messages.ts:40` | The grid can't shrink past “{label}” / a button. Move or remove it first. | The grid can't shrink past “{tile}”. Move or remove it first. | Added by hand: nested template |
| `src/lib/editor/messages.ts:46` | Button names can be up to 16 characters. | Tile labels can be up to 16 characters. | |
| `src/lib/editor/messages.ts:48` | Give the button a name. | Give the Tile a label. | |
| `src/lib/editor/messages.ts:50` | Page names can be up to 24 characters. | Page names can be up to 24 characters. | |
| `src/lib/editor/messages.ts:52` | Give the page a name. | Give the Page a name. | |
| `src/lib/editor/messages.ts:54` | Layout names can be up to 24 characters. | Layout names can be up to 24 characters. | |
| `src/lib/editor/messages.ts:56` | Give the layout a name. | Give the Layout a name. | |
| `src/lib/editor/messages.ts:58` | A page grid can have 1 to 8 rows and 1 to 8 columns. | A Page grid has 1 to 8 rows and 1 to 8 columns. | |
| `src/lib/editor/messages.ts:60` | This layout is too big for the deck. Remove some buttons or pages. | This Layout is too big for {device}. Remove some Tiles or Pages. | |
| `src/lib/editor/messages.ts:62` | Remove tabs, line breaks, and other special characters. | Remove tabs, line breaks and other special characters. | |
| `src/lib/editor/messages.ts:64` | A button goes to a page that no longer exists. | A Tile goes to a Page that no longer exists. | |
| `src/lib/editor/messages.ts:67` | Pick a key from the list. | Pick a key from the list. | |
| `src/lib/editor/messages.ts:69` | The deck can't take that change. | That change breaks a Layout rule. See Details. | |
| `src/lib/editor/messages.ts:75` | “{name}” isn't a TouchDeck layout file, so nothing changed. | “{file}” isn't a Layout file. Nothing changed. | |
| `src/lib/editor/messages.ts:76` | “{name}” has a problem the deck can't take, so nothing changed. | “{file}” breaks a Layout rule. Nothing changed. See Details. | |

### Page tabs

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/PageTabs.svelte:25` | Pages | Pages | aria-label |
| `src/lib/editor/PageTabs.svelte:41` | AI usage | Usage | Tag on a Usage page tab |
| `src/lib/editor/PageTabs.svelte:46` | A layout can have up to {n} pages / Add a page | A Layout holds up to {n} Pages / Dropped | Tooltip when full. The other tooltip repeated the label |
| `src/lib/editor/PageTabs.svelte:48` | Add page | Add Page | |

### Tile details

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/TileProps.svelte:33` | Previous | Previous | Built-in icon name |
| `src/lib/editor/TileProps.svelte:34` | Play and pause | Play and pause | |
| `src/lib/editor/TileProps.svelte:35` | Next | Next | |
| `src/lib/editor/TileProps.svelte:36` | Speaker off | Speaker off | |
| `src/lib/editor/TileProps.svelte:37` | Speaker, quiet | Volume low | |
| `src/lib/editor/TileProps.svelte:38` | Speaker, loud | Volume high | |
| `src/lib/editor/TileProps.svelte:54` | Button | Tile | Kicker |
| `src/lib/editor/TileProps.svelte:57` | Remove | Remove | |
| `src/lib/editor/TileProps.svelte:61` | Name on the button | Label | |
| `src/lib/editor/TileProps.svelte:63` | {length} of {max} characters. Long names get smaller on the deck. | {length} of {max} characters. Long labels shrink on {device}. | |
| `src/lib/editor/TileProps.svelte:69` | The deck can't show {chars}. It draws an empty box instead. | {device} can't show {chars} and draws an empty box instead. | |
| `src/lib/editor/TileProps.svelte:73` | When pressed | When tapped | |
| `src/lib/editor/TileProps.svelte:80` | Picture | Icon | |
| `src/lib/editor/TileProps.svelte:82` | Symbol | Dropped | The icon picker lists the built-in icons with the rest |
| `src/lib/editor/TileProps.svelte:88` | None, show the name only | No icon | |
| `src/lib/editor/TileProps.svelte:93` | Picture from the gallery | Dropped | Goes with the Symbol list |
| `src/lib/editor/TileProps.svelte:101` | Position and size | Position and size | |
| `src/lib/editor/TileProps.svelte:103` | Row | Row | |
| `src/lib/editor/TileProps.svelte:104` | Column | Column | |
| `src/lib/editor/TileProps.svelte:105` | Rows tall | Height in rows | |
| `src/lib/editor/TileProps.svelte:106` | Columns wide | Width in columns | |
| `src/lib/editor/TileProps.svelte:108` | On the deck you can also drag the button, or drag its corner dot. With the button selected, the arrow keys move it and Shift with an arrow key resizes it. | You can also drag the Tile or its corner dot. Arrow keys move the selected Tile, and Shift with an arrow key resizes it. | "On the deck" meant the preview |

### Layout file checks

These messages appear only under Details, after a sentence from "Layout rule messages". They stay as they are, because they name the exact field that broke the rule.

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/validate.ts:89` | {place} is not well-formed Unicode | Unchanged | Details only |
| `src/lib/editor/validate.ts:96` | {place} has the member name {name}, which is not well-formed Unicode | Unchanged | Details only |
| `src/lib/editor/validate.ts:134` | must be {expected} (type) | Unchanged | Details only |
| `src/lib/editor/validate.ts:142` | must have required property '{field}' (required) {field} | Unchanged | Details only |
| `src/lib/editor/validate.ts:145` | must NOT have additional properties (additionalProperties) {name} | Unchanged | Details only |
| `src/lib/editor/validate.ts:155` | must not contain U+0000 to U+001F or U+007F (pattern) | Unchanged | Details only |
| `src/lib/editor/validate.ts:160` | must NOT have fewer than 1 characters (minLength) | Unchanged | Details only |
| `src/lib/editor/validate.ts:166` | must NOT have more than {max} characters (maxLength) | Unchanged | Details only |
| `src/lib/editor/validate.ts:167` | must hold a character that is not whitespace (pattern) | Unchanged | Details only |
| `src/lib/editor/validate.ts:172` | must match pattern "^[a-z0-9_]{1,15}$" (pattern) | Unchanged | Details only |
| `src/lib/editor/validate.ts:178` | must be >= {min} (minimum) | Unchanged | Details only |
| `src/lib/editor/validate.ts:179` | must be <= {max} (maximum) | Unchanged | Details only |
| `src/lib/editor/validate.ts:183` | must be equal to one of the allowed values (enum) | Unchanged | Details only |
| `src/lib/editor/validate.ts:194` | must NOT have duplicate items (uniqueItems) | Unchanged | Details only |
| `src/lib/editor/validate.ts:201` | must match exactly one schema in oneOf (oneOf) | Unchanged | Details only |
| `src/lib/editor/validate.ts:236` | must be null, a built-in lv: name, or file: and 16 lowercase hex digits (pattern) | Unchanged | Details only |
| `src/lib/editor/validate.ts:249` | must match exactly one schema in oneOf (oneOf) | Unchanged | Details only |
| `src/lib/editor/validate.ts:263` | must NOT have more than {max} items (maxItems) | Unchanged | Details only |
| `src/lib/editor/validate.ts:270` | must be equal to constant (const) | Unchanged | Details only |
| `src/lib/editor/validate.ts:273` | must be equal to one of the allowed values (enum) | Unchanged | Details only |
| `src/lib/editor/validate.ts:281` | must have 1 to {max} items, not {n} (minItems, maxItems) | Unchanged | Details only |
| `src/lib/editor/validate.ts:290` | layout compact UTF-8 JSON is {n} bytes; maximum is {max} bytes | Unchanged | Details only |
| `src/lib/editor/validate.ts:295` | duplicate id "{id}" at {location}; first used at {first} | Unchanged | Details only |
| `src/lib/editor/validate.ts:306` | page "{page}" tile "{tile}" references missing page "{target}" | Unchanged | Details only |
| `src/lib/editor/validate.ts:309` | page "{page}" tile "{tile}" exceeds grid bounds {rows}x{cols} | Unchanged | Details only |
| `src/lib/editor/validate.ts:321` | page "{page}" tiles "{a}" and "{b}" overlap | Unchanged | Details only |
| `src/lib/editor/validate.ts:383` | {place} repeats the member name {name} | Unchanged | Details only |
| `src/lib/editor/validate.ts:386` | {place} is written as {token}; numbers must be JSON integers | Unchanged | Details only |
| `src/lib/editor/validate.ts:401` | file is not well-formed UTF-8 | Unchanged | Details only |
| `src/lib/editor/validate.ts:404` | JSON nests deeper than {n} levels | Unchanged | Details only |

### Default and starter Layout names

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/validate.ts:612` | Tile | Tile | Label of a new Tile |
| `src/lib/editor/validate.ts:620` | New layout / Page 1 | New Layout / Page 1 | Names of a Blank Layout |
| `src/lib/editor/validate.ts:649` | My buttons | Starter | Name of the Starter Layout |
| `src/lib/editor/validate.ts:654` | Media | Media | Page name |
| `src/lib/editor/validate.ts:659` | Previous | Previous | Tile label |
| `src/lib/editor/validate.ts:660` | Play/Pause | Play/Pause | |
| `src/lib/editor/validate.ts:661` | Next | Next | |
| `src/lib/editor/validate.ts:662` | Mute | Mute | |
| `src/lib/editor/validate.ts:663` | Volume down | Volume down | |
| `src/lib/editor/validate.ts:664` | Volume up | Volume up | |
| `src/lib/editor/validate.ts:669` | Shortcuts | Shortcuts | Page name |
| `src/lib/editor/validate.ts:674` | Copy | Copy | |
| `src/lib/editor/validate.ts:675` | Paste | Paste | |
| `src/lib/editor/validate.ts:676` | Cut | Cut | |
| `src/lib/editor/validate.ts:677` | Select all | Select all | |
| `src/lib/editor/validate.ts:678` | Undo | Undo | |
| `src/lib/editor/validate.ts:679` | Redo | Redo | |
| `src/lib/editor/validate.ts:680` | Save | Save | |
| `src/lib/editor/validate.ts:681` | Find | Find | |
| `src/lib/editor/validate.ts:682` | Screenshot | Screenshot | |
| `src/lib/editor/validate.ts:683` | Show desktop | Show desktop | |

### Device settings (moves to the Device panel)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/settings/Settings.svelte:35` | {n} hour(s) | {n} hr | Added by hand, inside the timeout hint |
| `src/lib/settings/Settings.svelte:36` | {n} minute(s) | {n} min | Added by hand |
| `src/lib/settings/Settings.svelte:37` | {n} second(s) | {n} s | Added by hand |
| `src/lib/settings/Settings.svelte:47` | Showing your deck's current settings. | Dropped | The panel always shows live values |
| `src/lib/settings/Settings.svelte:49` | Couldn't read your deck's settings. Reconnect it, then try again. | Couldn't read {device}'s settings. Unplug it and plug it back in. | |
| `src/lib/settings/Settings.svelte:65` | Saved. Restart the deck to turn its screen. / Saved on your deck. | Dropped / Saved | Orientation asks to restart before it saves. See Device panel |
| `src/lib/settings/Settings.svelte:68` | Couldn't save to your deck. Check that it's connected, then try again. | Couldn't save to {device}. Check that it's plugged in, then retry. | |
| `src/lib/settings/Settings.svelte:79` | Your deck is restarting. Connect again once its buttons are back. | Restarting… | It reconnects on its own |
| `src/lib/settings/Settings.svelte:81` | Couldn't restart your deck. Unplug it and plug it back in instead. | Couldn't restart {device}. Unplug it and plug it back in. | |
| `src/lib/settings/Settings.svelte:90` | Your deck | {device} | Title of the Device panel |
| `src/lib/settings/Settings.svelte:91` | These settings are saved on the deck itself, so they stay with it on any computer. | Saved on {device}, so they follow it to any PC. | |
| `src/lib/settings/Settings.svelte:95` | Connect your deck to change its settings. | Plug in {device} to change its settings. | |
| `src/lib/settings/Settings.svelte:100` | Screen | Screen | |
| `src/lib/settings/Settings.svelte:103` | Turn the screen off after | Screen off after | |
| `src/lib/settings/Settings.svelte:106` | seconds | s | Added by hand |
| `src/lib/settings/Settings.svelte:108` | That's {duration} without a touch. Touch the screen to wake it. | {duration} without a touch. Touch to wake. | |
| `src/lib/settings/Settings.svelte:111` | Enter a whole number of seconds from {min} to {max}, which is one day. | Enter whole seconds from 1 to 86,400 (one day). | |
| `src/lib/settings/Settings.svelte:117` | Which way it stands | Orientation | |
| `src/lib/settings/Settings.svelte:118` | Landscape is wide, portrait is tall. Each way keeps its own buttons, and the deck restarts to turn. | Each orientation keeps its own Layout. Changing it restarts {device}. | |
| `src/lib/settings/Settings.svelte:121` | Orientation | Orientation | |
| `src/lib/settings/Settings.svelte:123` | Landscape, wide | Landscape | |
| `src/lib/settings/Settings.svelte:124` | Portrait, tall | Portrait | |
| `src/lib/settings/Settings.svelte:130` | Save to deck | Dropped | The panel saves each change on its own |
| `src/lib/settings/Settings.svelte:132` | Restart deck now | Restart | |
| `src/lib/settings/Settings.svelte:139` | Details | Details | |

### Troubleshooting

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/settings/Troubleshooting.svelte:15` | Device | Product | The `hello` product id, not the glossary's Device |
| `src/lib/settings/Troubleshooting.svelte:16` | Link version | Protocol | |
| `src/lib/settings/Troubleshooting.svelte:17` | Software build ID | Firmware build | |
| `src/lib/settings/Troubleshooting.svelte:18` | Software version | Firmware version | |
| `src/lib/settings/Troubleshooting.svelte:19` | Running from | Running from | |
| `src/lib/settings/Troubleshooting.svelte:20` | Sends keys through | Sends keys through | |
| `src/lib/settings/Troubleshooting.svelte:21` | Safe mode | Safe mode | |
| `src/lib/settings/Troubleshooting.svelte:22` | Layout on screen | Current Layout | |
| `src/lib/settings/Troubleshooting.svelte:28` | yes / no | Yes / No | Added by hand |
| `src/lib/settings/Troubleshooting.svelte:50` | Troubleshooting | Troubleshooting | |
| `src/lib/settings/Troubleshooting.svelte:51` | What TouchDeck knows about your deck, and tools to check that buttons reach this computer. You won't need these day to day. | Device details, and tools to check that taps reach this PC. | |
| `src/lib/settings/Troubleshooting.svelte:56` | Connection | Connection | |
| `src/lib/settings/Troubleshooting.svelte:58` | Connected through {the practice deck built into TouchDeck / USB port {port}}. | {device} is on {port} / the demo Device. | The mock Device is the "demo Device" |
| `src/lib/settings/Troubleshooting.svelte:62` | No deck is connected. Plug it in with a USB cable, then choose Connect at the top right. | No Device connected. Plug one in with a USB cable. | |
| `src/lib/settings/Troubleshooting.svelte:66` | Last connection message | Last connection error | |
| `src/lib/settings/Troubleshooting.svelte:79` | App build {sha} | Build {sha} | Moves to Settings, About |
| `src/lib/settings/Troubleshooting.svelte:85` | Test your keys | Key test | |
| `src/lib/settings/Troubleshooting.svelte:86` | Click in the box, then press buttons on your deck. Each key the deck sends to this computer shows up here. | Click the box, then tap Tiles on {device}. Each key it sends to this PC shows here. | |
| `src/lib/settings/Troubleshooting.svelte:93` | Button activity | Action log | |
| `src/lib/settings/Troubleshooting.svelte:94` | Buttons that open apps or run commands, and what happened each time one was pressed. | What happened each time a Tile opened an app or ran a command. | |
| `src/lib/settings/Troubleshooting.svelte:101` | Deck console | Firmware console | |
| `src/lib/settings/Troubleshooting.svelte:102` | Send a text command straight to the deck's built-in console. For example, version or layout state. | Send a command to {device}'s console, like version or layout state. | |
| `src/lib/settings/Troubleshooting.svelte:107` | > {text} | > {text} | Echo of a sent command, added by hand |
| `src/lib/settings/Troubleshooting.svelte:110` | Nothing yet. Send a command below. / Connect your deck to use the console. | Nothing yet. Send a command below. / Plug in {device} to use the console. | |
| `src/lib/settings/Troubleshooting.svelte:116` | version | version | Placeholder, added by hand |
| `src/lib/settings/Troubleshooting.svelte:120` | Console command | Console command | aria-label |
| `src/lib/settings/Troubleshooting.svelte:123` | Send | Send | |

### Connection menu (dropped)

Detection is automatic, so the connection pill and its panel go away. Status words move to the Home card.

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/shell/ConnectionMenu.svelte:11` | Deck connected / Connecting… / Connection lost / Deck not connected | Connected / Connecting… / Not connected / Not connected | Home card status. With no Disconnect, lost and unplugged read the same |
| `src/lib/shell/ConnectionMenu.svelte:50` | Your deck | Dropped | Panel aria-label |
| `src/lib/shell/ConnectionMenu.svelte:53` | Your deck is connected | Dropped | The Home card says Connected |
| `src/lib/shell/ConnectionMenu.svelte:54` | It's using this computer's {practice deck / USB port {port}}. | Dropped | The port shows in Troubleshooting |
| `src/lib/shell/ConnectionMenu.svelte:60` | Details | Dropped | |
| `src/lib/shell/ConnectionMenu.svelte:65` | Connect your deck | Plug in your Device | Becomes the Home empty state title |
| `src/lib/shell/ConnectionMenu.svelte:66` | Plug your TouchDeck into this computer with a USB cable, then choose it below. | Connect an Open TouchDeck to this PC with a USB data cable. | Home empty state body |
| `src/lib/shell/ConnectionMenu.svelte:75` | Practice deck / TouchDeck | Demo Device / Dropped | The card shows the Device's name |
| `src/lib/shell/ConnectionMenu.svelte:79` | Disconnect | Dropped | The Companion holds every Device |
| `src/lib/shell/ConnectionMenu.svelte:85` | Connecting… / Connect | Dropped | Connect buttons go away |
| `src/lib/shell/ConnectionMenu.svelte:96` | Connect | Dropped | |
| `src/lib/shell/ConnectionMenu.svelte:100` | No deck found. Check the USB cable, then look again. | Dropped | Covered by the Home empty state |
| `src/lib/shell/ConnectionMenu.svelte:109` | Looking… / Look again | Dropped | The Companion watches ports all the time |
| `src/lib/shell/ConnectionMenu.svelte:116` | Connection details | Connection help | Home empty state button to Troubleshooting |

### CLIProxyAPI Integration (was AI usage)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/sources/api.ts:75` | never | Never | Added by hand |
| `src/lib/sources/api.ts:77` | {n} s ago | {n} s ago | |
| `src/lib/sources/api.ts:79` | {n} min ago / {n} h ago | {n} min ago / {n} hr ago | |
| `src/lib/sources/CliProxy.svelte:46` | Couldn't copy. Select the account in the table and copy it instead. | Couldn't copy. Select the account in the table and copy it. | |
| `src/lib/sources/CliProxy.svelte:54` | Connect to your CLIProxyAPI hub | CLIProxyAPI | Integration card title |
| `src/lib/sources/CliProxy.svelte:55` | CLIProxyAPI is a hub that signs in to your AI accounts. TouchDeck asks it, read-only, how much of each limit is used. | Shows how much of each AI account's limit is used, read from your CLIProxyAPI hub. | |
| `src/lib/sources/CliProxy.svelte:58` | Read usage from the hub | Turn on CLIProxyAPI | Switch aria-label |
| `src/lib/sources/CliProxy.svelte:66` | Off. Turn it on and save to start reading usage. | Off | Status word |
| `src/lib/sources/CliProxy.svelte:68` | Checking the hub for the first time… | Checking… | Status word |
| `src/lib/sources/CliProxy.svelte:70` | Reading {n} account(s). Last checked {ago}. | Connected. {n} accounts, checked {ago}. | |
| `src/lib/sources/CliProxy.svelte:72` | The hub hasn't answered {n} times in a row, so the deck greys out its AI usage buttons until it does. | Can't connect. The hub missed {n} checks, so its Tiles are greyed out. | |
| `src/lib/sources/CliProxy.svelte:74` | The hub didn't answer the last {n} check(s). TouchDeck keeps trying. | The hub missed the last {n} checks. Retrying. | |
| `src/lib/sources/CliProxy.svelte:81` | Hub address | Hub address | |
| `src/lib/sources/CliProxy.svelte:82` | https://hub.example.com | https://hub.example.com | Placeholder, added by hand |
| `src/lib/sources/CliProxy.svelte:85` | Check every | Check every | |
| `src/lib/sources/CliProxy.svelte:88` | seconds | s | Added by hand |
| `src/lib/sources/CliProxy.svelte:93` | This hub address starts with http:// and isn't on this computer, so the management key travels unencrypted. Use an https:// address to protect it. | This address isn't on this PC and uses http://, so the key is sent unencrypted. Use https://. | |
| `src/lib/sources/CliProxy.svelte:96` | Management key | Management key | |
| `src/lib/sources/CliProxy.svelte:98` | Saved, ends in {hint} / Not set yet / Management key | Saved · ends in {last4} / Not set / Management key | Placeholder, then aria-label |
| `src/lib/sources/CliProxy.svelte:99` | Save key | Save | |
| `src/lib/sources/CliProxy.svelte:100` | Forget key | Forget | |
| `src/lib/sources/CliProxy.svelte:102` | Kept in your computer's secure password store, never in a file. Last 4 characters: {hint or none} | Stored in Windows Credential Manager, never in a file. | The placeholder already shows the last 4 |
| `src/lib/sources/CliProxy.svelte:107` | Check now | Check now | |
| `src/lib/sources/CliProxy.svelte:108` | Each check asks your AI services through the hub, so TouchDeck waits at least 15 seconds between them. | Checks are at least 15 s apart, because each one queries your AI services. | |
| `src/lib/sources/CliProxy.svelte:114` | Account, Service, Plan, 5-hour limit, 7-day limit, Status | Account, Service, Plan, 5-hour limit, 7-day limit, Status | Table headers |
| `src/lib/sources/CliProxy.svelte:120` | Copy {account} / Copied / Copy | Copy {account} / Copied / Copy | aria-label, then button |
| `src/lib/sources/CliProxy.svelte:123` | — | — | Empty plan cell, added by hand |
| `src/lib/sources/CliProxy.svelte:129` | resets now / resets in {time} / reset time unknown | Resets now / Resets in {time} / Reset time unknown | |
| `src/lib/sources/CliProxy.svelte:136` | {status} | Active / Disabled / Unavailable / Error | Badge, from the hub's raw word. See the Rust table |
| `src/lib/sources/CliProxy.svelte:138` | {n} worked · {n} failed · {n} in 10 min | {n} OK · {n} failed · {n} in 10 min | |
| `src/lib/sources/CliProxy.svelte:145` | Limits show the share already used. Copy an account to show only that account on a button. Gemini and some other services only report request counts for now. | Limits show the share used. Copy an account to show only that account on a Tile. Gemini and some other services report request counts only. | |

### About and app updates (was This app)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/updates/AppUpdates.svelte:11` | unknown | Unknown | Version when it can't be read, added by hand |
| `src/lib/updates/AppUpdates.svelte:22` | This app | About | |
| `src/lib/updates/AppUpdates.svelte:23` | You have TouchDeck {version}. Your deck's own software updates separately, under Deck software. | Open TouchDeck {version} | Firmware lives in the Device panel |
| `src/lib/updates/AppUpdates.svelte:26` | Checking for updates… / Check for updates | Checking… / Check for updates | |
| `src/lib/updates/AppUpdates.svelte:29` | You’re on the latest app version. | Up to date | |
| `src/lib/updates/AppUpdates.svelte:31` | {error message} | One of the update dialog's failure strings | Chosen by error kind. See the Rust table |
| `src/lib/updates/AppUpdates.svelte:33` | Offline or a release may be unavailable. You can keep using this version and try again later. | You're offline. Connect to the internet, then retry. | |
| `src/lib/updates/AppUpdates.svelte:36` | Version {version} | Open TouchDeck {version} | Update dialog title |
| `src/lib/updates/AppUpdates.svelte:37` | No release notes provided. | No release notes. | |
| `src/lib/updates/AppUpdates.svelte:39` | Download update | Update | |
| `src/lib/updates/AppUpdates.svelte:40` | Downloads and verifies the signed installer. TouchDeck stays open. | You can keep working. | Shown while downloading |
| `src/lib/updates/AppUpdates.svelte:42` | Downloading update: {kb} KB of {total} KB | Downloading… {pct}% | Lines 42 and 43 |
| `src/lib/updates/AppUpdates.svelte:45` | The signature is verified before installation is offered. | Dropped | The signature check is in the failure message when it fails |
| `src/lib/updates/AppUpdates.svelte:47` | Download verified. Save or export any unsaved layout edits before installing. | Ready to restart | |
| `src/lib/updates/AppUpdates.svelte:48` | Installation closes TouchDeck. Open it again when the installer finishes; your saved settings stay in place. | Open TouchDeck restarts to install. Your Layouts and settings stay. | |
| `src/lib/updates/AppUpdates.svelte:49` | Disconnect the deck and wait for transfers to finish before installing. | Wait for {device} to finish sending. | Only while a transfer runs |
| `src/lib/updates/AppUpdates.svelte:50` | Install and close TouchDeck | Restart | |
| `src/lib/updates/AppUpdates.svelte:52` | Starting the installer. TouchDeck will close; installation progress appears in the installer. | Restarting to install… | |

### Layout screen (was Buttons)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/routes/Editor.svelte:47` | TouchDeck layout | Open TouchDeck Layout | File type in the save dialog |
| `src/routes/Editor.svelte:58` | Saved “{name}”. | Exported “{file}” | |
| `src/routes/Editor.svelte:60` | Couldn't save the file. Pick another folder and try again. | Couldn't export. Pick another folder, then retry. | |
| `src/routes/Editor.svelte:85` | Your deck is plugged in. Connect to send your buttons to it. | Dropped | Connect buttons go away |
| `src/routes/Editor.svelte:87` | Connecting… / Connect | Dropped | |
| `src/routes/Editor.svelte:91` | No deck connected. | {device} isn't connected. | Banner, lines 91 and 92 |
| `src/routes/Editor.svelte:92` | Plug your TouchDeck into this computer with a USB cable. / Looking for your deck… You can still set up buttons now. | Plug it in with a USB cable. You can still edit its Layout. | |
| `src/routes/Editor.svelte:94` | Looking… / Look again | Dropped | |
| `src/routes/Editor.svelte:95` | Help | Dropped | Troubleshooting is in Settings |
| `src/routes/Editor.svelte:104` | {n} page(s) · {n} button(s) | {n} Pages · {n} Tiles | Added by hand |
| `src/routes/Editor.svelte:106` | Mode | Mode | aria-label |
| `src/routes/Editor.svelte:107` | Edit | Edit | |
| `src/routes/Editor.svelte:108` | Try it like the deck (P) / Try it | Preview (P) / Preview | Tooltip, then button |
| `src/routes/Editor.svelte:110` | Which way the deck stands | Orientation | aria-label |
| `src/routes/Editor.svelte:116` | Wide / Tall | Dropped | The tooltips repeated the labels |
| `src/routes/Editor.svelte:121` | Landscape / Portrait | Landscape / Portrait | |
| `src/routes/Editor.svelte:131` | Send these buttons to your deck / Connect your deck first | Send this Layout to {device} / {device} isn't connected | Tooltip |
| `src/routes/Editor.svelte:132` | Sending… / Send to deck | Sending… / Send to Device | |
| `src/routes/Editor.svelte:140` | Your deck has “{name}” | {device} has “{layout}” | Lines 140 to 143 |
| `src/routes/Editor.svelte:143` | ({n} pages, {n} buttons). Replace the {orientation} layout here with it? | ({n} Pages, {n} Tiles). Replace the {orientation} Layout here with it? | |
| `src/routes/Editor.svelte:145` | Replace | Replace | |
| `src/routes/Editor.svelte:146` | Keep mine | Keep mine | |
| `src/routes/Editor.svelte:155` | Add button | Add Tile | |
| `src/routes/Editor.svelte:178` | Swipe sideways or press ← and → to change pages, and tap a button to see it react, exactly as on the deck. | Swipe or press ← and → to change Pages. Tap a Tile to see it react. | |
| `src/routes/Editor.svelte:181` | Actual pixels | Actual size | |
| `src/routes/Editor.svelte:184` | Click a button to change it, or an empty square to add one. Drag to move, and drag the corner dot to resize. | Click a Tile to edit it, or an empty square to add one. Drag to move, and drag the corner dot to resize. | |
| `src/routes/Editor.svelte:186` | An AI usage page fills itself with your accounts once AI usage is set up. These rows are examples. | A Usage page lists your accounts once CLIProxyAPI is set up. These rows are examples. | |
| `src/routes/Editor.svelte:191` | Details | Details | Inspector aria-label |
| `src/routes/Editor.svelte:201` | Set up your buttons | Start a Layout | Empty state title |
| `src/routes/Editor.svelte:202` | Choose what each button on your TouchDeck does. You'll see every page exactly as the deck will show it. | Choose what each Tile on {device} does. | |
| `src/routes/Editor.svelte:205` | Start from your deck | Load from {device} | |
| `src/routes/Editor.svelte:206` | Load the buttons your deck has now. / Connect your deck to load its buttons. | Use the Layout {device} has now. / Plug in {device} to load its Layout. | |
| `src/routes/Editor.svelte:209` | Start with examples | Starter Layout | |
| `src/routes/Editor.svelte:210` | Media controls and everyday shortcuts, ready to change. | Media controls and everyday shortcuts, ready to change. | |
| `src/routes/Editor.svelte:213` | Start blank | Blank Layout | |
| `src/routes/Editor.svelte:214` | One empty page with room for 12 buttons. | One empty Page with room for 12 Tiles. | |
| `src/routes/Editor.svelte:217` | Open a layout file… | Import | |
| `src/routes/Editor.svelte:225` | Details | Details | |

### Settings page sections

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/routes/Settings.svelte:15` | Your deck / Screen and orientation | Device / Dropped | Becomes the Device panel, opened from the Device's name. Its title is the name |
| `src/routes/Settings.svelte:16` | Deck software / Update or repair the deck | Firmware / Dropped | A section of the Device panel. Repair moves to Troubleshooting |
| `src/routes/Settings.svelte:17` | This app / Version and updates | About / Update | Updates become the Update icon and dialog, plus Check for updates in About |
| `src/routes/Settings.svelte:18` | Troubleshooting / Connection, testing, and logs | Troubleshooting / Dropped | |
| `src/routes/Settings.svelte:23` | Settings | Settings | aria-label |

### Firmware update and repair (was Deck software)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/routes/Update.svelte:9` | Rebooting, Confirming | Unchanged | Raw step names, shown only under Details |
| `src/routes/Update.svelte:10` | Recovering | Unchanged | Details only |
| `src/routes/Update.svelte:25` | Failed: {reason} | Unchanged | Details only |
| `src/routes/Update.svelte:30` | Checking | Unchanged | Details only |
| `src/routes/Update.svelte:31` | Checking the update file… | Checking the file… | |
| `src/routes/Update.svelte:32` | Sending | Unchanged | Details only |
| `src/routes/Update.svelte:33` | Sending the new software to your deck… {pct}% | Updating… {pct}% | |
| `src/routes/Update.svelte:34` | Recovering | Unchanged | Details only |
| `src/routes/Update.svelte:35` | Repairing your deck… {pct}% | Repairing… {pct}% | |
| `src/routes/Update.svelte:36` | Rebooting | Unchanged | Details only |
| `src/routes/Update.svelte:37` | Restarting your deck… | Restarting… | |
| `src/routes/Update.svelte:38` | Confirming | Unchanged | Details only |
| `src/routes/Update.svelte:39` | Making sure the new software started… | Checking that it started… | |
| `src/routes/Update.svelte:40` | Done | Unchanged | Details only |
| `src/routes/Update.svelte:41` | Firmware updated / Done. Your deck is running the new software. | Updated to {version} | The first is the raw message it matches |
| `src/routes/Update.svelte:42` | Already on this build / Your deck already has this version, so nothing changed. | Already on Firmware {version} | |
| `src/routes/Update.svelte:43` | Done. {message}. | Done. {message}. | Fallback for an unknown message |
| `src/routes/Update.svelte:44` | Failed | Unchanged | Details only |
| `src/routes/Update.svelte:45` | The new software didn't start, so your deck went back to its old version. It keeps working. Try another update file, or choose Repair. | The new Firmware didn't start, so {device} went back to {version}. It still works. Retry, or repair it from Troubleshooting. | |
| `src/routes/Update.svelte:46` | Your deck didn't come back after restarting. Unplug it and plug it back in. If its buttons don't return, choose Repair. | {device} didn't come back after restarting. Unplug it and plug it back in. If its screen stays blank, repair it from Troubleshooting. | |
| `src/routes/Update.svelte:47` | The update didn't finish, and your deck keeps its old software. Check the file and the cable, then try again. | Couldn't update. {device} still runs {version}. Check the cable, then retry. | |
| `src/routes/Update.svelte:62` | Firmware bundle exceeds 6 MB | That file is over 6 MB, so it isn't a Firmware file. | |
| `src/routes/Update.svelte:82` | Deck software | Firmware | Device panel section |
| `src/routes/Update.svelte:83` | Install a new version of the software that runs on your deck. Your buttons and settings stay on it. | Your Layout and settings stay on {device} when you update. | |
| `src/routes/Update.svelte:88` | Choose the update file | Dropped | Firmware updates are one click. Install from file moves to Troubleshooting |
| `src/routes/Update.svelte:89` | It ends in .tdfw and comes with each TouchDeck release. | A .tdfw file from an Open TouchDeck release. | Troubleshooting, install from file |
| `src/routes/Update.svelte:91` | Choose file… | Choose file | |
| `src/routes/Update.svelte:96` | Keep the deck plugged in, then update | Keep {device} plugged in. | |
| `src/routes/Update.svelte:97` | It takes about a minute. The deck restarts once on its own. | Takes about a minute. {device} restarts once. | |
| `src/routes/Update.svelte:99` | Update deck | Update | |
| `src/routes/Update.svelte:108` | Details | Details | |
| `src/routes/Update.svelte:115` | Update log | Dropped | Merged into Details |
| `src/routes/Update.svelte:123` | Repair the deck | Repair Firmware | Troubleshooting |
| `src/routes/Update.svelte:124` | If an update stopped partway or the deck won't start, Repair puts the software back through the deck's USB loader. Your buttons stay. | If an update stopped partway or {device} won't start, Repair reinstalls Firmware through the USB loader. The Layout stays. | |
| `src/routes/Update.svelte:128` | USB port | Port | |
| `src/routes/Update.svelte:129` | COM3 | COM3 | Placeholder |
| `src/routes/Update.svelte:131` | Repair deck | Repair | |
| `src/routes/Update.svelte:133` | TouchDeck fills in the port of the connected deck. Choose the update file above first. | Choose the Firmware file first. The port fills in on its own. | |

### AI usage page (dropped into Integrations)

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/routes/Usage.svelte:56` | AI usage | CLIProxyAPI | Card on the Integrations page |
| `src/routes/Usage.svelte:58` | Show how much of your AI plans you've used, right on the deck. Add a button that shows AI usage, or an AI usage page, and TouchDeck keeps | Dropped | The card's description covers it (lines 58 and 59) |
| `src/routes/Usage.svelte:59` | its numbers up to date while it runs. | Dropped | |
| `src/routes/Usage.svelte:63` | Sending usage to your deck: {n} update(s) so far. / Your deck isn’t connected, so nothing is being sent. | Dropped / Dropped | Home shows which Devices are connected |
| `src/routes/Usage.svelte:72` | Which buttons to fill | Dropped | The Companion fills the CLIProxyAPI Tiles in every Device's own Layout |
| `src/routes/Usage.svelte:73` | TouchDeck fills the AI usage buttons and pages of the layout on your deck. To fill the ones in a saved layout file instead, name the file here. | Dropped | Same reason |
| `src/routes/Usage.svelte:76` | Layout file, optional | Dropped | Same reason |
| `src/routes/Usage.svelte:79` | Leave empty to use the buttons on your deck | Dropped | Same reason |
| `src/routes/Usage.svelte:92` | That layout couldn't be read, so no buttons are filled. Check the file name. | Dropped | Same reason |
| `src/routes/Usage.svelte:95` | “{name}” has {n} AI usage button(s) and {n} AI usage page(s). | Dropped | Same reason |
| `src/routes/Usage.svelte:97` | No layout yet. Connect your deck, or name a layout file. | Dropped | Same reason |
| `src/routes/Usage.svelte:100` | Couldn't send the numbers to your deck: {error} | Couldn't send values to {device}. See Details. | |
| `src/routes/Usage.svelte:104` | Save | Save | |
| `src/routes/Usage.svelte:105` | Saved / You have changes to save | Saved / Unsaved changes | |
| `src/routes/Usage.svelte:111` | Something went wrong with AI usage. Try again, or restart TouchDeck. | Couldn't load CLIProxyAPI settings. Retry, or restart Open TouchDeck. | |
| `src/routes/Usage.svelte:112` | Details | Details | |

### Messages from the Companion and Firmware that reach the UI

History: the `src-tauri/` locations in this table cite the removed Tauri crate and record what the migration replaced; those files no longer exist. Today these Rust strings showed in the UI, most under Details and some as the main text. The Companion API gives each error a code and a message (spec, "API contract"). The UI shows the "New" sentence for the code and puts the raw message under Details. Locations are relative to `app/`.

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src-tauri/src/app_updates.rs:59` | {updater plugin error, network kind} | Couldn't download the update. Check your internet connection, then retry. | Kind `network` |
| `src-tauri/src/app_updates.rs:59` | {updater plugin error, signature kind} | The download didn't match the project's signing key, so it wasn't installed. Retry later. | Kind `signature` |
| `src-tauri/src/app_updates.rs:59` | {updater plugin error, metadata kind} | The update server sent something Open TouchDeck can't read. Retry later. | Kind `metadata` |
| `src-tauri/src/app_updates.rs:109` | TouchDeck app installation is starting; deck operations are blocked | Restarting to install… | Kind `busy` |
| `src-tauri/src/app_updates.rs:121` | Wait for deck operations to finish before installing the app update | Wait for {device} to finish sending. | Kind `busy` |
| `src-tauri/src/app_updates.rs:201` | {installer launch error} | Couldn't start the installer. Retry, or download it from the releases page. | Kind `installer` |
| `src-tauri/src/app_updates.rs:236` | An app update operation is already running | Already updating | Kind `busy` |
| `src-tauri/src/app_updates.rs:250` | Desktop app updates are currently packaged for Windows x64 only | Updates are for Windows x64 only. Download the new version from the releases page. | Kind `unsupported` |
| `src-tauri/src/app_updates.rs:264` | This candidate has no release signing public key. Install an official release to enable app updates | This build can't update itself. Install an official release. | Kind `unconfigured` |
| `src-tauri/src/app_updates.rs:274` | App update download must use HTTPS | The update server sent something Open TouchDeck can't read. Retry later. | Kind `metadata` |
| `src-tauri/src/app_updates.rs:313` | Check for a newer app version before downloading | Unchanged | Programming guard, never shown in normal use |
| `src-tauri/src/app_updates.rs:339` | Installer execution is disabled on this platform or mock session | Unchanged | Test sessions only |
| `src-tauri/src/app_updates.rs:347` | Disconnect the deck before installing the app update | Wait for {device} to finish sending. | Kind `busy` |
| `src-tauri/src/app_updates.rs:356` | Download and verify the app update before installing | Unchanged | Programming guard |
| `src-tauri/src/app_updates.rs:364` | {internal error} | Couldn't update. Retry, or restart Open TouchDeck. | Kind `internal` |
| `src-tauri/src/commands.rs:100` | the icon cache is not set up | Icons didn't load. Restart Open TouchDeck, then retry. | |
| `src-tauri/src/commands.rs:200` | firmware update in progress | {device} is updating. Wait for it to finish. | |
| `src-tauri/src/commands.rs:312` | no deck is connected | {device} isn't connected. | |
| `src-tauri/src/commands.rs:322` | no deck is connected | {device} isn't connected. | |
| `src-tauri/src/commands.rs:409` | no icon file for {id} | Icon missing. Choose another. | |
| `src-tauri/src/commands.rs:430` | the deck's layout is not UTF-8 | {device}'s Layout has a problem Open TouchDeck can't read. Nothing changed. | |
| `src-tauri/src/commands.rs:507` | unknown mock action {action} | Unchanged | Test sessions only |
| `src-tauri/src/commands.rs:729` | no such tile in the deck's layout | Ignored, not in {device}'s Layout | Action log |
| `src-tauri/src/commands.rs:736` | no layout to look the tile up in | Ignored, not in {device}'s Layout | Action log |
| `src-tauri/src/commands.rs:750` | {path}: {error} | Couldn't open {path}. | Action log, raw error under Details |
| `src-tauri/src/commands.rs:786` | {command}: {error} | Couldn't run {program}. | Action log, raw error under Details |
| `src-tauri/src/commands.rs:883` | no approval request {id} is waiting | Couldn't send your answer. Tap the Tile again. | |
| `src-tauri/src/commands.rs:905` | unknown answer {answer} | Unchanged | Programming guard |
| `src-tauri/src/commands.rs:910` | allowed once | Allowed once | Action log detail |
| `src-tauri/src/commands.rs:911` | always allowed | Always allowed | Action log detail |
| `src-tauri/src/commands.rs:914` | ran, but the approval was not saved: {error} | Ran, but Open TouchDeck couldn't save “Always allow”. It asks again next time. | |
| `src-tauri/src/commands.rs:1045` | firmware update already in progress | {device} is updating. Wait for it to finish. | |
| `src-tauri/src/commands.rs:1060` | select the mock port | Unchanged | Test sessions only |
| `src-tauri/src/commands.rs:1066` | ROM recovery requires real hardware | Unchanged | Test sessions only |
| `src-tauri/src/sources.rs:64` | no reason given | Unchanged | Details only |
| `src-tauri/src/sources.rs:185` | the management key is empty | Enter the management key. | |
| `crates/td-link/src/update.rs:81` | no update state reply | Couldn't update. {device} still runs {version}. Check the cable, then retry. | Same sentence for lines 81 to 138 |
| `crates/td-link/src/update.rs:105` | bad update transfer id | (as above) | Details only |
| `crates/td-link/src/update.rs:109` | bad update slot | (as above) | Details only |
| `crates/td-link/src/update.rs:116` | image has too many chunks | That file is over 6 MB, so it isn't a Firmware file. | |
| `crates/td-link/src/update.rs:138` | chunk {n} acknowledgement timeout | (as above) | Details only |
| `crates/td-link/src/update.rs:177` | rolled back | The new Firmware didn't start, so {device} went back to {version}. It still works. Retry, or repair it from Troubleshooting. | |
| `crates/td-link/src/update.rs:186` | confirm timeout | {device} didn't come back after restarting. Unplug it and plug it back in. If its screen stays blank, repair it from Troubleshooting. | |
| `crates/td-link/src/update.rs:189` | port return timeout | (as for confirm timeout) | |
| `crates/td-link/src/update.rs:214` | port is not a TouchDeck | That port isn't an Open TouchDeck Device. | |
| `crates/td-link/src/update.rs:218` | Already on this build | Already on Firmware {version} | |
| `crates/td-link/src/update.rs:227` | Firmware updated | Updated to {version} | |
| `crates/td-link/src/update.rs:321` | unplugged | {device} was unplugged during the update. Plug it back in. If its screen stays blank, repair it from Troubleshooting. | |
| `crates/td-link/src/session.rs:71` | no reply within {ms} ms | {device} didn't answer. Unplug it and plug it back in. | |
| `crates/td-link/src/session.rs:72` | link closed: {reason} | Lost {device}. Check the USB cable. It reconnects on its own. | |
| `crates/td-link/src/session.rs:74` | a transfer is already running | {device} is busy. Wait a moment, then retry. | |
| `crates/td-link/src/session.rs:75` | a layout state reply is still pending | {device} is busy. Wait a moment, then retry. | |
| `crates/td-link/src/session.rs:76` | write failed: {error} | Lost {device}. Check the USB cable. It reconnects on its own. | |
| `crates/td-link/src/session.rs:485` | read failed: {error} | Lost {device}. Check the USB cable. It reconnects on its own. | |
| `crates/td-link/src/session.rs:500` | heartbeat failed: {error} | Lost {device}. Check the USB cable. It reconnects on its own. | |
| `crates/td-link/src/session.rs:30` | the Device dropped the session | Lost {device}. It reconnects on its own. | |
| `crates/td-link/src/recover.rs:60` | recovery port not found | Can't find {device}'s USB loader. Unplug it, plug it back in, then retry. | |
| `crates/td-link/src/recover.rs:62` | recovery requires USB-Serial/JTAG | Repair works only over the board's built-in USB port. | |
| `crates/td-link/src/recover.rs:65` | recovery requires Espressif USB-Serial/JTAG | Repair works only over the board's built-in USB port. | |
| `crates/td-link/src/bundle.rs:37` | bundle must contain exactly the manifest and three firmware files | That file isn't a Firmware file for this Device. | Same sentence for every bundle.rs line, raw text under Details |
| `crates/td-link/src/bundle.rs:43` | missing {name} | (as above) | |
| `crates/td-link/src/bundle.rs:59` | unsupported TouchDeck bundle | (as above) | |
| `crates/td-link/src/bundle.rs:69` | manifest needs exactly one {name} | (as above) | |
| `crates/td-link/src/bundle.rs:73` | offset needs 0x | (as above) | |
| `crates/td-link/src/bundle.rs:78` | unsafe offset for {name} | (as above) | |
| `crates/td-link/src/bundle.rs:100` | application descriptor does not match TouchDeck manifest | (as above) | |
| `crates/td-link/src/bundle.rs:118` | unsafe partition table: {name} | (as above) | |
| `crates/td-host/src/icons.rs:64` | index.json is missing | Icons didn't load. Restart Open TouchDeck, then retry. | |
| `crates/td-host/src/icons.rs:67` | index.json: {error} | Icons didn't load. Restart Open TouchDeck, then retry. | |
| `crates/td-host/src/icons.rs:146` | not a TDI1 icon file | Icon missing. Choose another. | |
| `crates/td-host/src/sources/cliproxy.rs:23` | TLS certificate verification failed; for a private CA, add it to the Windows certificate store | Can't verify the hub's certificate. If it uses a private CA, add the CA to Windows. | |
| `crates/td-host/src/sources/cliproxy.rs:98` | disabled | Disabled | Account badge |
| `crates/td-host/src/sources/cliproxy.rs:102` | unavailable | Unavailable | Account badge. "active" and "error" become Active and Error |
| `crates/td-host/src/sources/cliproxy.rs:178` | not an auth-files reply | The hub sent a reply Open TouchDeck can't read. Check the hub address. | |
| `crates/td-host/src/sources/cliproxy.rs:225` | now | Resets now | |
| `crates/td-host/src/sources/cliproxy.rs:248` | malformed usage reply | The hub sent a reply Open TouchDeck can't read. Check the hub address. | |
| `crates/td-host/src/sources/cliproxy.rs:422` | HTTP {code} | The hub answered with error {code}. | 401 and 403 read "The hub refused the management key. Check it, then save it again." |
| `crates/td-host/src/sources/cliproxy.rs:423` | request timed out (15 s) | The hub didn't answer within 15 s. | |
| `crates/td-host/src/sources/cliproxy.rs:426` | hub redirect refused | The hub tried to redirect. Use its final address. | |
| `crates/td-host/src/sources/cliproxy.rs:428` | hub host could not be resolved | Can't find that hub address. Check it for typos. | |
| `crates/td-host/src/sources/cliproxy.rs:433` | connection refused | The hub refused the connection. Check that it's running. | |
| `crates/td-host/src/sources/cliproxy.rs:435` | hub request failed | Can't connect to the hub. | |
| `crates/td-host/src/sources/cliproxy.rs:570` | no management key is set | Enter the management key. | |
| `crates/td-host/src/sources/cliproxy.rs:596` | missing auth index | The hub sent a reply Open TouchDeck can't read. Check the hub address. | |
| `crates/td-host/src/sources/mod.rs:335` | {path}: {error} | Dropped | Goes with the layout file field |
| `crates/td-host/src/sources/mod.rs:340` | invalid layout | Dropped | Goes with the layout file field |

### Strings the check script flags that no one sees

The coverage script below matches any literal that reads like words. These are internal keys, so they need no copy.

| Location | Old | New | Note |
| --- | --- | --- | --- |
| `src/lib/editor/icons.ts:18` | TDI1 | Internal | Icon file magic |
| `src/lib/editor/layoutStore.ts:103` | no layout is open | Internal | Refusal key, shown as "That change breaks a Layout rule." |
| `src/lib/editor/layoutStore.ts:177` | no free cell on this page | Internal | Refusal key for "This Page is full." |
| `src/lib/editor/messages.ts:34` | no free cell on this page | Internal | Refusal key, row above has its sentence |
| `src/lib/editor/messages.ts:37` | a layout keeps at least 1 page | Internal | Refusal key, row above has its sentence |
| `src/lib/editor/validate.ts:437` | no page {page} | Internal | Refusal key |
| `src/lib/editor/validate.ts:438` | page {page} is a dashboard | Internal | Refusal key |
| `src/lib/editor/validate.ts:448` | outside the {rows}x{cols} grid | Internal | Refusal key for "That doesn't fit." |
| `src/lib/editor/validate.ts:492` | outside the {rows}x{cols} grid | Internal | Refusal key |

## Checking coverage

A script lists the string literals and text nodes in every non-test `.svelte` and `.ts` file under `app/src`, keeps the ones that read as words, and checks that each appears in a row above with its `file:line`. Single lowercase words, such as "seconds" or "never", don't pass its filter, so the rows marked "Added by hand" cover them. When a UI ticket changes a string, it updates the row here.

### This PC Integration

| Element | String |
|---|---|
| Integration name | This PC |
| Unsupported platform | This PC needs Windows. |
| Unavailable counters | Unavailable: {names}. |
| Preset tile, `cpu_pct` | CPU usage |
| Preset tile, `ram_pct` | RAM usage |
| Preset tile, `ram_used_bytes` | RAM used |
| Preset tile, `ram_total_bytes` | RAM total |
| Preset tile, `gpu_pct` | GPU usage |
| Preset tile, `disk_active_pct` | Disk activity |
| Preset tile, `disk_read_bytes_s` | Disk read |
| Preset tile, `disk_write_bytes_s` | Disk write |
| Preset tile, `network_receive_bytes_s` | Network receive |
| Preset tile, `network_send_bytes_s` | Network send |

## Animated GIF Tiles

| State | Copy |
| --- | --- |
| Add Tile section | Animation |
| Add Tile choice | GIF |
| Add Tile choice, second line | Plays an animated GIF |
| Shows field, third option | GIF |
| Field label | GIF |
| Field, a GIF Tile without a GIF yet | No GIF |
| Field, a GIF from before this session | GIF |
| Choose | Choose GIF |
| Replace | Change GIF |
| Fit field | Fit |
| Fit options, `contain` and `cover` | Whole GIF, Fill Tile |
| Waiting for the GIF ahead of it | Waiting for another GIF… |
| Preparing | Preparing GIF… |
| Stop preparing | Cancel |
| Fitted | Fitted to {device}: {width} × {height}, {fps} fps. |
| Fitted, a GIF with one frame | Fitted to {device}: {width} × {height}. |
| Too long | This GIF is too long for {device}. Choose a shorter GIF. |
| Too large | This GIF is too large for {device}. Choose a smaller GIF. |
| Too short | This GIF is too short for {device}. Choose a GIF that plays for at least {limit} ms. |
| Page budget | This Page has too many animations for {device}. Remove an animated Tile. |
| Device Page budget refusal | This Page has too many animations. Remove an animated Tile. |
| Storage full | {device} is out of space. Remove unused Assets, then retry. |
| Asset store full | This PC's Asset store is full. Retry in an hour, or remove Tiles you don't use and send the Layout. |
| Decoded source too large | This GIF needs more than 64 MiB to decode all its frames. Choose a smaller or shorter GIF. |
| Preparation failed | Couldn't prepare this GIF. Choose another GIF. |
| Can't play GIFs (`asset.unsupported`) | {device} can't play this GIF. Update its Firmware, then retry. |
| Two render Jobs already run (`limits.exceeded`) | Other Assets are still being prepared. Wait a moment, then retry. |
| Device not connected | Couldn't prepare this GIF. Check that {device} is plugged in, then retry. |
| Refit with no file, a GIF from before this session | Choose this GIF again to refit it. |
| Failure buttons | Retry, Details |
| Unsupported | Needs Firmware {x.y} |

The Companion API returns these rows without `for {device}`, because an error
body has no Device name; the app adds it. Put decoder and wire reasons under Details. App previews show a paused poster
when reduced motion is requested.

`asset.too_large` picks its row by `details.field`: `gif_frames` and
`source_frames` get Too long, `decoded_rgba` gets Decoded source too large, and
every other field Too large. A link refusal reaches the app typed: `size` as
`asset.too_large`, `format` as `asset.unsupported`, `memory` as Too large and
`full` as Storage full. An upload over 16 MiB (`request.too_large`) gets Too large.
Unsupported is the Tile badge on the canvas for a Device whose Firmware can't play
GIF Tiles; Add Tile offers GIF only to a Device whose capabilities have `anim`.

## Graph Tiles

| State | Copy |
| --- | --- |
| Field | Shows |
| Choices | Label (Value on a Live tile), Graph |
| Stream field | Stream |
| Stream hint | Updates about once a second. |
| Stream gone | {stream} (unknown) |
| No numeric streams | Set up an Integration to draw a graph. |
| Unsupported | Needs Firmware {x.y}. |
| No fresh value (on the Device and in the preview) | -- |

The stream picker groups streams by Integration and names each by its Preset
tile. A stale graph keeps its last line at 60% opacity, as on the Device.

## Custom Integration

| Element | String |
|---|---|
| Integration | Custom |
| Description | Read values from HTTP or a command approved on this PC. |
| Enabled | Turn on Custom |
| Config | Custom sources (JSON) |
| Refresh floor | Checks are at least 1 s apart. |
| Invalid JSON | Enter valid JSON. |
| Number Presets, in source order | {name} value, {name} graph |
| Approval | Waiting for approval on this PC. |
| Approval denied | Denied on this PC. |
| Approval expired | Approval expired. |
| Corrupt settings file | Couldn't read custom.json. Kept it as custom.corrupt.json and started empty. (The name is custom.corrupt-2.json, -3 and so on when a backup already exists.) |
| Unreadable settings file | Couldn't read custom.json. Started empty. |
| Missing headers | Enter the secret headers. |
| Vault failure | Credential store unavailable. |
| Transport | Use HTTPS for secret headers. |
| Invalid address | Check the HTTP address. |
| Access refused | The server refused access. Check the secret headers. |
| Timeout | The server didn't answer within 10 s. |
| HTTP size | The HTTP reply is over 1 MiB. |
| TLS | Can't verify the server certificate. Check the TLS fingerprint. |
| Redirect | The server redirected. Use its final address. |
| Request failed | Couldn't fetch the value. Check the HTTP address. |
| Invalid value | Couldn't read the value. Check the value path and kind. |
| Start failure | Couldn't start the command. |
| Output failure | Couldn't read command output. |
| Wait failure | Couldn't finish the command. |
| Exit failure | The command failed. Check the program and arguments. |
| Encoding | Command output must be UTF-8. |
| Command size | Command output is over 64 KiB. |
| Command timeout | The command exceeded 10 s and was stopped. |

## Volume and Media

| Context | Copy |
|---|---|
| Integration names | Volume; Media |
| Presets | Volume bar; Now playing |
| Idle Tile | Nothing playing |
| Unsupported Tile | Needs Firmware {x.y} |
| Volume status / failed action | No playback device. |
| Non-Windows Volume status | Volume needs Windows. |
| Non-Windows Media status | Media needs Windows. |
| Failed media action, idle | Nothing is playing. |
| Failed media action, timeout or false | The player didn't respond. |
| ControlError UnknownAccount, UnknownAction, NotSettable or OutOfRange | This Tile's control isn't available. |
| Successful mute action | Mute changed. |
| Successful volume gesture | Volume changed. |
| Successful previous action | Previous track. |
| Successful play_pause action | Playback changed. |
| Successful next action | Next track. |
| Cover storage failure | {device} is out of space for cover art. Remove unused Assets, then retry. |
| Too many cover Tiles (`layout.live_image_limit`) | This Layout has too many Now playing Tiles for {device}. Remove one, then send it again. |
| Volume description | Shows and sets this PC's volume. |
| Media description | Shows what's playing on this PC, with play, pause and skip. |
| Enabled | Turn on Volume; Turn on Media |
| Shows field, a Tile its Integration offers as a bar | Value, Graph, Bar |
| Drag switch | Drag to change |
| Drag switch hint | Slide a finger along the bar on {device} to set it. |
| Shows field, a now-playing Tile | Now playing |
| Shows hint, a now-playing Tile | The title, artist and cover from Media. Tap its buttons to play, pause or skip. |
| Preset tile the Device can't draw yet, accessible name | Add {preset} to Layout. Needs Firmware {x.y} |

The Companion's API error leaves ` for {device}` out of the Too many cover Tiles row, and its delivery status
starts the cover storage row with "The Device"; the app puts in the Device's name. Integration action-log entries use kind integration and status ok or failed.
A completed drag has one entry. An idle Media Integration is healthy; backend
failures use error status. Wire/decoder details belong under Details.

The app preview draws a bar Tile at 64 and a now-playing Tile with a sample
track and a drawn stand-in cover, because covers never leave the Companion. The
Drag switch shows only when the bar's stream is settable. Without the Firmware,
Add Tile still offers the Volume bar and Now playing Presets with Needs Firmware
{x.y} where their art goes, and places them; the Tile shows its label and the
badge until the Device can draw it.

## Controls need Send to Device

A connected Device whose Companion reports `control_layout` as `no-pushed-copy` or `differs`, with `has_controls`, shows these. `matches`, a Layout without controls, a Device that isn't connected and `unavailable` show nothing new. The notices name the action by its button, **Send to Device**, never as "a Send". A Send to Device that fails keeps the notice and shows the failure with Details. After one that succeeds, Checking controls… stays until a fresh assessment arrives: `matches` or a Layout without controls ends it quietly, `no-pushed-copy` or `differs` brings the notice back, and `read-failed` or `unsupported` shows "Couldn't check" until a later assessment decides, so a Send to Device never looks confirmed when nothing was checked.

| Element | String |
| --- | --- |
| Home, under the status | Controls need Send to Device |
| Home button | Review Layout |
| Home button, accessible name | Review Layout for {device} |
| Editor notice title | Controls need Send to Device |
| Editor notice, no copy on this PC | These Tiles show live values, but their controls aren't set up on this PC. |
| Editor notice, the Layout differs | The Layout on {device} differs from the one this PC sent. |
| Editor notice, after either line | Sending replaces the Layout on {device} and lets its Tiles control the Integrations shown here. |
| Editor notice, the editor shows the other orientation | {device} shows its {orientation} Layout. Switch to {Orientation} to review it. |
| Editor notice button | Review Layout |
| Editor notice button, while loading | Loading… |
| Editor notice button, the other orientation | Switch to {Orientation} |
| After Send to Device, until the Companion reports a match | Checking controls… |
| After Send to Device, the Companion couldn't read the Layout (`read-failed`) | Couldn't check {device}'s controls. Unplug it and plug it back in. |
| After Send to Device, the Firmware can't report its Layout (`unsupported`) | Couldn't check {device}'s controls. Update its Firmware. |
| Inspector section | Controls |
| Inspector count | {n} Tiles |
| Inspector hint | Tiles on every Page that control an Integration. Select one to change it. |
| Inspector, no control Tiles | No Tile on this Layout controls an Integration. |
| Row, where | {page} Page |
| Row, the Integration's own account | Default account |
| Row, an Integration this PC doesn't have | Unknown Integration “{id}” |
| Row, an account this PC doesn't have | Unknown account “{id}” |
| Row, controls | Drag to change, Mute, Previous track, Play or pause, Next track, joined by “ · ” |

`{orientation}` is lowercase in a sentence and capitalized on the button, as the Orientation options are. Unknown names show the Device's text in quotes, so a Tile label never stands in for an Integration.

## Brandfetch logos

Settings, Brandfetch logos, and the icon picker's Brandfetch section (#127). `{keychain}` is "Windows Credential Manager" on Windows, "your system keychain (GNOME Keyring or KWallet)" on Linux and "your system keychain" elsewhere, as in the Secret field hint. The Field buttons, the empty field and the Forget confirm follow the Integrations Secret field.

### Settings, Brandfetch logos

| Element | String |
| --- | --- |
| Settings entry | Brandfetch logos |
| Settings entry, on | On. The icon picker searches Brandfetch with your client ID. |
| Settings entry, off | Off. Add your own Brandfetch keys to search brand logos. |
| Section title | Brandfetch logos |
| Lede | Search brand logos in the icon picker with your own Brandfetch account. It stays off until you add a client ID. |
| Reading | Reading the Brandfetch settings… |
| Keys heading | Keys |
| Client ID label | Client ID |
| Client ID hint | Turns on logo search in the icon picker. Searches go from this window straight to Brandfetch. Adding or forgetting it reloads the window. |
| API key label | API key |
| API key hint | Turns a picked logo into a Tile icon. Each pick uses one Brand API request; Test key is free. |
| Field, empty | Not set |
| API key, saved | Set |
| Field buttons | Save, Cancel, Replace, Forget |
| Test key button | Test key |
| Test key worked | Brandfetch accepted the key. |
| Forget confirm, client ID | Forget the client ID? Logo search stops until you add it again. |
| Forget confirm, API key | Forget the API key? Picking Brandfetch logos stops until you add it again. |
| Logos heading | Logos on this PC |
| Logo count, none | No Brandfetch logos yet |
| Logo count, all fresh | {n} logo, all up to date; {n} logos, all up to date |
| Logo count, some due | {n} logos; 1 needs a refresh; {n} logos; {m} need a refresh |
| Refresh line | Each logo is fetched again from day 25, so no cached copy is older than 30 days. |
| Expired line | {n} went 30 days without a refresh, so this PC deleted their cached copy. Their Tiles keep showing them; pick them again to refresh them. |
| Stored note heading | How the key is stored |
| Stored note | The API key is stored in {keychain}, never in a file. Open TouchDeck never shows it again and sends it only to Brandfetch's Brand API, when you pick a logo, test the key or a logo is refreshed. |
| Notes heading | Good to know |
| Notes | You use your own Brandfetch account and keys. Open TouchDeck ships none. / Logos stay the trademarks of their brand owners. / Brandfetch lets apps keep a logo for 30 days. This PC fetches each one again before then; the free developer plan includes about 100 Brand API requests. |
| Portal link | Get a client ID and API key from Brandfetch |
| Settings couldn't be read | Couldn't read the Brandfetch settings. Restart Open TouchDeck, then retry. |
| Companion unreachable | Can't reach the Companion. Check that Open TouchDeck is running in the tray, then retry. |
| Client ID rejected (`request.invalid`) | That doesn't look like a client ID. Copy it from the Brandfetch developer portal. |
| API key rejected (`request.invalid`) | That doesn't look like an API key. Copy it from the Brandfetch developer portal. |
| Save or Forget failed | Couldn't save the Brandfetch settings. Retry, or restart Open TouchDeck. |
| Test key, no key saved (`brandfetch.no_key`) | Save an API key first. |
| Test key, refused (`brandfetch.key_refused`) | Brandfetch refused the key. Replace it with the API key from the Brandfetch developer portal. |
| Test key, used up (`brandfetch.quota`) | Your Brandfetch API requests are used up. Check your plan in the Brandfetch developer portal. |
| Test key, any other failure | Brandfetch didn't answer. Check the internet connection, then retry. |
| No keychain, Windows (`brandfetch.no_keychain`) | Windows Credential Manager isn't available. Restart Open TouchDeck and try again. |
| No keychain, Linux | No system keychain is available. Start GNOME Keyring or KWallet and try again. |
| No keychain, elsewhere | No system keychain is available. Start it and try again. |
| Failure link | Details |

A failed save, Forget or Test key shows the sentence above and the Companion's message under Details. The Companion's `brandfetch.no_keychain` message, "The system keychain isn't available.", names no platform; the UI picks the row for the platform `system.get` reports.

### Icon picker, Brandfetch section

Shown only while a client ID is saved.

| Element | String |
| --- | --- |
| Section title | Brandfetch |
| Cell, accessible name | {name}, Brandfetch |
| Cell tooltip | {name} · Brandfetch |
| A result without a name | Its domain stands in for {name} |
| Getting a logo | Getting {name}… |
| Recent icon name | Brandfetch logo |
| Credits, with a client ID | "· Brandfetch (your keys)" after the Icon pack credits, as the Icon picker Credits row shows |
| Search refused (401, 403) | Brandfetch refused the client ID. Check it in Settings, Brandfetch logos. |
| Too many searches (429) | Too many Brandfetch searches for now. Wait a few minutes, then retry. |
| Search the query can't run (400) | Brandfetch couldn't search for that. Try a shorter brand name. |
| Search failed, another status | Brandfetch couldn't answer. Retry in a moment. |
| Search offline | Can't reach Brandfetch. Check the internet connection, then retry. |
| Search failure button | Retry Brandfetch |
| Pick, no key saved (`brandfetch.no_key`) | Add your Brandfetch API key in Settings, Brandfetch logos, then retry. |
| Pick, key refused (`brandfetch.key_refused`) | Brandfetch refused the API key. Check it in Settings, Brandfetch logos. |
| Pick, used up (`brandfetch.quota`) | Your Brandfetch API requests are used up. Check your plan in the Brandfetch developer portal. |
| Pick, unknown brand (`brandfetch.not_found`) | Brandfetch has no details for {name}. Choose another result. |
| Pick, no usable logo (`brandfetch.no_logo`) | Brandfetch has no logo for {name} that this PC can download. Choose another result. |
| Pick, no keychain (`brandfetch.no_keychain`) | The No keychain rows above, for the platform |
| Pick, too many at once (`limits.exceeded`) | Other icons are still being prepared. Wait a moment, then retry. |
| Pick, any other failure | Couldn't get {name} from Brandfetch. Check the internet connection, then retry. |
| Pick failure buttons | Retry, Details |

The Tile action destination field is **Send to**, with **This PC (Companion)**
and **Bluetooth computer or phone**. Offer it only for shortcuts and media keys
when the Device advertises the `bluetooth` sink. A Bluetooth shortcut hint ends with
“on the Bluetooth computer or phone”.

Imported Bluetooth Tiles keep **Send to** visible on Devices without that destination.
Show the inline error: “This Device can't send to Bluetooth. Choose This PC (Companion).”
