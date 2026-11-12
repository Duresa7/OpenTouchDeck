Superseded by ADR 0011.

# A Rust Companion behind a thin Electron window

The app splits in two: a web UI, and the Companion, a background program built in Rust from the existing `td-link`, `td-host` and `td-core` crates. The Companion owns USB, actions, Integrations and the keychain, and serves the UI through one local API on `127.0.0.1`. The Electron window and a browser tab are both clients of that API.

The window moves from Tauri to Electron at the maintainer's request. The Rust stays instead of being rewritten in Node, because it already holds the tested serial protocol, Firmware flashing with rollback, key injection and app-icon extraction, and it has direct access to the Windows APIs for volume, media and CPU.

## Considered options

- **Stay on Tauri.** A smaller install and no updater migration. The maintainer chose Electron.
- **Electron with the Rust rewritten in Node.** The most work and risk, and it would need native add-ons for keys, volume, media and icons.
- **A browser-only mode over Web Serial.** Without a program on the PC it can't type, launch apps or read PC data, so it can't run a Device.

## Consequences

Electron's main process holds no product logic. Anything that needs the OS goes in the Companion, so the browser gets it too. Don't add Node IPC handlers for features.

The Companion starts when the user signs in to Windows and lives in the tray, so Devices keep working with no window open. Its local API listens on `127.0.0.1` only and requires a pairing token from every client, including the Electron window, so other websites in the user's browser can't drive it. Access from phones or other devices on the network was declined for now.
