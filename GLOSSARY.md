# Open TouchDeck

A touchscreen deck for your desk, and the desktop app that designs and drives it.

## Language

### Hardware

**Device**:
One physical Open TouchDeck. It has a name the user gives it, a hardware identity that never changes, and its own Layout.
_Avoid_: deck, your deck, board

**New board**:
A board connected to the computer that doesn't run Open TouchDeck Firmware yet. It becomes a Device once Firmware is installed and it's named.
_Avoid_: blank deck, recovery deck

**Firmware**:
The software running on a Device. It belongs to that Device and is updated from the app.
_Avoid_: deck software

### Computer

**Companion**:
The background program on the user's computer that runs the Devices, Integrations and actions, and serves the app's UI to the desktop window or a browser.
_Avoid_: daemon, service, backend, host

**Carrier**:
How the Companion reaches a Device: USB today, Wi-Fi and Bluetooth later. Every carrier moves the same link frames, so everything the Companion does with a Device works the same on each.
_Avoid_: transport, connection type

### Layout

**Layout**:
All the Pages of one Device, with their Tiles.
_Avoid_: config, profile, buttons

**Page**:
One screen of Tiles on a Device.

**Tile**:
One slot on a Page. It shows something and can run an action when tapped.
_Avoid_: key, slot

**Live tile**:
A Tile whose content keeps changing with data from an Integration, such as a graph or now playing.
_Avoid_: widget, dynamic tile

**Preset tile**:
A ready-made Tile that an Integration offers, such as a CPU graph, a volume bar or now playing.
_Avoid_: widget

**Asset**:
An icon, image or animation prepared for a Device and stored on it.
_Avoid_: resource, media file

**Icon pack**:
An installable, licensed set of icons that the icon picker searches offline. A person downloads a curated one, or imports an Iconify JSON file, on the Icon packs page in Integrations, and turns all of them on or off with its switch; nothing is installed by default.
_Avoid_: icon set, icon library, icon theme

**Pushed copy**:
The Layout this Companion last sent to a Device, kept on the PC. Controls on a Tile only act when the Device shows exactly its Pushed copy.
_Avoid_: trusted copy, authority copy

### Integrations

**Integration**:
A connection to something outside the app that supplies data or accepts commands, such as a CLIProxyAPI hub, the PC or media playback.
_Avoid_: source, plugin

### Connections

**Carrier**:
The way a Device and the Companion reach each other: USB, Wi-Fi or Bluetooth. A Device keeps the same identity on every Carrier and uses one at a time.
_Avoid_: transport, connection type

**Pairing**:
A Device's approval of one Companion to reach it over Wi-Fi or Bluetooth, given once by entering the code the Device shows. It lasts until someone removes it.
_Avoid_: bond (see Keyboard host), trust, registration

**Bluetooth keyboard**:
The radio build's way for a Device to type keys and media keys on other computers and phones over Bluetooth, with no Companion involved. It is off until the person turns it on.
_Avoid_: BLE HID, Bluetooth mode

**Keyboard host**:
A computer or phone the Device's Bluetooth keyboard types on. A person adds one by typing on it the 6-digit code the Device shows, and it stays until someone forgets it. The UI says "computer or phone" and uses the verb "add".
_Avoid_: bond, paired device, Pairing (that is a Companion's approval)
