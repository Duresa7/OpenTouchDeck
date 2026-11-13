# One Rust program runs the Companion and a Slint UI

Decided 2026-11-12. Supersedes ADR 0001 and ADR 0002. ADR 0012 supersedes the Electron and React timing below: the lines it changes are marked.

Open TouchDeck 2.0 is one Rust program: the Companion and a Slint UI run in the same process. Use Slint's Fluent style and AccessKit for Narrator, muda for native menus, and the winit backend for the HWND. The Microsoft `windows` crate handles DWM Mica, toasts, taskbar integration and a fixed AppUserModelID (AUMID). File and folder dialogs use `rfd`.

The reasons are memory use, a Windows 11 look and feel, and one implementation language for the app and Companion. The maintainer's measurements put the Electron window at about 300 MB PSS across nine processes and the Rust Companion alone at about 42 MB in a debug build. These are starting measurements, not a benchmark of the new UI or a promised saving. Slint's Fluent style, accessibility integration and expected memory use motivated the choice; we have not established a comparative ranking of Rust toolkits.

Slint is used under GPL-3.0: the shipped combined app is GPL-3.0, while our own source files stay Apache-2.0. [Slint permits this arrangement](https://slint.dev/community). This preserves the OSI-licence basis for applying to [SignPath Foundation](https://signpath.org/terms), rather than choosing Slint's royalty-free licence, which is not OSI-approved. Eligibility is not an acceptance or signing commitment.

## Considered options

- **C# and WinUI 3.** [Microsoft's recommendation](https://learn.microsoft.com/windows/apps/winui) for new native Windows apps, but it adds a second language and a process boundary to the Rust Companion. WinUI 3 is Windows-only; Linux is next.
- **Both Electron and native apps.** Rejected in favour of maintaining one program.
- **Keep Electron.** Rejected over measured RAM and the non-native feel.
- **Other Rust GUI toolkits or a webview shell.** Slint was preferred for the reasons above; there is no claim that every alternative was benchmarked.

## Consequences

2.0 supports Windows 11 only. The installer blocks Windows 10 with a clear message; 1.x stays on Releases for Windows 10. Linux comes next, then macOS.

Development happens on main beside the current app. The Electron shell, React UI and Companion local web API, including browser access, are frozen now and deleted at cutover. _(Superseded by ADR 0012: the Electron shell and React UI left main for the `release/1.x` branch, and only the local web API stays until cutover.)_ There is no interim app release. PR #341 is closed; issues #326–#329 are re-scoped for Slint. The Companion entry in GLOSSARY.md keeps its current UI-serving definition until cutover. _(Superseded by ADR 0012: the Companion no longer serves a UI, and its glossary entry says so.)_

Tray left-click opens the app. Start at sign-in is a checkbox, on by default; a one-time notice explains close-to-tray. Use Mica, the Windows accent colour, Segoe UI Variable, contrast themes, text scaling and a minimum width of about 500 effective pixels. Draft Fluent-style app icon options for the maintainer to choose, then provide every required size. The [Windows guide](../research/windows-11-native.md) records integration work and proof requirements.

Ship a per-user MSI built with WiX. Releases are unsigned for now, but include VERSIONINFO and a code signing policy page, and build only on GitHub-hosted runners. ADR 0005 still holds: keep the existing minisign updater key. Its old v0.2.0 migration decision does not describe this migration: 1.x moves automatically to 2.0 through a bridge in the 1.x updater, subject to the Windows 11 requirement. Update checks are opt-out and documented on a PRIVACY page.

Native CI, a signing-ready release pipeline and faster PR checks accompany the rewrite. Prove Windows behaviour with screenshots and RAM measurements on GitHub-hosted Windows runners; the maintainer's PC is unavailable. Record runner OS/build and measurement method so results can be interpreted.

## Milestones

Tracked in #359.

- **M0:** groundwork.
- **M1:** vertical slice: tray, window inside the Companion, Home, About, Mica, accent, typography, contrast, text scaling, minimum size, Windows CI screenshots and RAM.
- **M2:** first-run setup and Settings.
- **M3:** Layout editor.
- **M4:** Integrations and the Device panel.
- **M5:** MSI, updater, release pipeline and migration.
- **M6:** delete Electron, React and the local web API; release 2.0. _(Superseded by ADR 0012: Electron and React are already gone from main. M6 deletes the local web API and releases 2.0.)_
