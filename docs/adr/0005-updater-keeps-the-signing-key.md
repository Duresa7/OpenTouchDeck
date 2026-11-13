# The Electron updater keeps the existing signing key

> Note added 2026-11-13: the key decision still stands; the Electron updater it names does not. Open TouchDeck 2.0 updates through `td-updater` and the MSI ([ADR 0011](0011-native-windows-app-in-rust-and-slint.md), [packaging guide](../packaging.md#the-updater-public-key)), which check the same key. Electron and electron-updater left `main` with [ADR 0012](0012-electron-and-react-leave-main-before-cutover.md) and remain on the `release/1.x` branch. The text below is unchanged from its decision date.

The Electron app keeps the existing updater key. There's no bridge release: v0.2.0 doesn't update itself to the Electron app, and its Layouts and settings don't carry over. The maintainer has the only v0.2.0 install and replaces it by hand (decided 2026-10-30).

The Electron updater checks that key's signature before installing. A stock electron-updater setup without a code-signing certificate checks only a checksum published next to the installer, so anyone who could change a GitHub release could ship an update. With the separate key, a hijacked release alone can't. Installers stay without an Authenticode signature until there's a certificate.

Never regenerate or replace the key. Every installed copy trusts it.
