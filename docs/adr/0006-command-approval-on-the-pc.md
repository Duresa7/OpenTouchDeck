# Command approvals happen on the PC, never through the API

When a Tile or the custom command Integration would start a program that isn't approved yet, the Companion shows a native Windows confirmation dialog that it owns. The dialog offers **Run once**, **Always allow** and **Deny**, with Deny as the default button. No API session, from the Electron window or a browser, can approve a command. The local API only lists pending approvals, read-only, and can ask the Companion to show a prompt again, so the UI can say "Waiting for approval on this PC".

The API can push a Layout, and a Layout can hold a command Tile. If a session could also answer the approval, any authentication bug in the API would let an attacker run programs as the user. With the dialog on the PC, the same bug lets an attacker change a Layout, and the user still sees the exact program, arguments and working folder in a window that no web page can draw or click.

Approval covers every action that starts a process: commands, program launches, and opening anything other than an `http`, `https` or `mailto` URL. An approval is tied to the exact program, arguments, working folder and shell, so changing any of them asks again. Sessions may remove an approval, because that only takes access away.

The same native dialog confirms Firmware writes that cannot be undone: installing on a New board, repair, downgrades and build switches. Every update from a picked `.tdfw` file also requires confirmation, regardless of its version or build. These files carry no signature. The dialog says the file isn't signed by Open TouchDeck and will replace the Device's Firmware. It names the target and its port, and shows the current Firmware version and build if known alongside the bundle's version and build. Consent binds to that target and bundle. The Job waits for confirmation, and declining or cancelling writes nothing. Same-or-newer updates from the bundle embedded in the app stay one click. While a native approval or Firmware confirmation dialog shows, the Companion suppresses key input from every Device so it cannot answer the dialog.

For 2.0, this consent requirement addresses the unsigned-file release blocker in [#422](https://github.com/Duresa7/OpenTouchDeck/issues/422). Bundle signing remains a follow-up.

## Considered options

- **Answer approvals in the UI, as today.** One click in the app the user is already looking at. It makes every API auth defect a remote code execution, and a browser tab is harder to trust than the window.
- **Answer in the UI, but only from the Electron window session.** Narrower, but the window loads the same web UI as a browser, so an XSS bug in the UI still approves commands.
- **Approve on the Device's screen.** The user is touching the Device anyway. But the Layout that draws the screen comes from the API, and a 5-inch Tile can't show a full command line.

## Consequences

The Companion needs a native dialog on Windows (a task dialog) and a fallback on Linux source builds. A tray program may not get the foreground on Windows, so the tray icon also shows a notification, and the UI's "Show prompt again" brings the dialog back.

Automated tests can't click a native dialog. The `test-hooks` build replaces the dialog with a scripted fake, and a test checks that release builds contain no route that answers an approval.

Program launches and file opens ran without approval in v0.2.0. Once this lands, the first press of such a Tile asks.
