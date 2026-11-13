# A Windows 11 native app

Research for the 2.0 rewrite in [ADR 0011](../adr/0011-native-windows-app-in-rust-and-slint.md), checked on 2026-11-12. This guide separates documented platform behaviour from project requirements and implementation gaps. Short quotes identify the source's wording; “not found” means the allowed primary sources did not establish the claim. Slint's `latest` Rust docs reported 1.18.1 during research; recheck against the version selected in M0.

The project decision is one Rust process containing the Companion and Slint, with Fluent styling, muda menus, `rfd` dialogs and Microsoft `windows` bindings. “Native” here does not mean WinUI controls: Slint supplies its own widgets. Microsoft calls [WinUI 3](https://learn.microsoft.com/windows/apps/winui) its “recommended native UI framework” for new Windows desktop apps; the project's Rust and future Linux requirements explain choosing Slint instead.

## Notification area

Use a permanent `guidItem` and `NIF_GUID` in `NOTIFYICONDATAW`, including subsequent modifications and deletion. Microsoft calls the GUID the “recommended method of identifying the icon”. Preserve the executable path across upgrades too: the documented preservation of icon settings requires both path and GUID to remain unchanged (with an exception for moves signed by the same company). This gives Windows a continuing identity for placement and visibility preferences, including the user's always-show choice; it does not let us force visibility. [NOTIFYICONDATAW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw).

Register `TaskbarCreated` on a top-level window. After Explorer recreates the taskbar, assume the old icon is gone and add it again. Microsoft says to “add them again”. This must also work while the main app window is hidden. [Taskbar creation notification](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar#taskbar-creation-notification).

After every `NIM_ADD`, call `NIM_SETVERSION` with `NOTIFYICON_VERSION_4`. Handle `NIN_KEYSELECT`, `NIN_SELECT` and `WM_CONTEXTMENU`; Space/Enter activation must work. Return focus with `NIM_SETFOCUS` after dismissing a tray menu. The API requires setting the version “every time” an icon is added. [Shell_NotifyIconW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw). Version 4 changes callback packing: use the event in `LOWORD(lParam)` and the anchor coordinates in `wParam`, rather than treating the whole `lParam` as a mouse message. [NOTIFYICONDATAW callback contract](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw#members).

Right-click opens a shortcut menu; left-click shows the content appropriate to the app. Microsoft explicitly allows “the program window itself”. Open TouchDeck chooses left-click to open or restore its window, right-click for Open, Start at sign-in and Quit. Use a DPI-aware icon loader: Microsoft's tray guidance recommends 16 and 32 px resources and `LoadIconMetric`. [Notification area guidance](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area).

Slint now documents a native [SystemTrayIcon](https://docs.slint.dev/latest/docs/slint/reference/window/systemtrayicon/), backed by `Shell_NotifyIcon` on Windows, with left-click callbacks and a right-click menu. Its public page does not establish a stable GUID or Explorer-restart contract. Those are proof obligations: verify the selected version, or implement the tray through `windows` and the selected muda menu integration. Do not infer complete Windows behaviour from the presence of a tray widget.

## App icons

Windows looks for an exact image size, otherwise the next larger image to downscale. Microsoft says “Including more icon sizes” reduces scaling. Its Windows 11 table gives these physical pixel sizes by surface and display scale. [Construct your Windows app's icon](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction).

| Surface | 100% | 125% | 150% | 200% | 250% | 300% | 400% |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Context menu, title bar, tray | 16 | 20 | 24 | 32 | 40 | 48 | 64 |
| Taskbar, search, Start all apps | 24 | 30 | 36 | 48 | 60 | 72 | 96 |
| Start pins | 32 | 40 | 48 | 64 | 80 | 96 | 256 |

For this app, include the union in the executable and installer `.ico`: **16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256**. This exceeds Microsoft's bare minimum of 16/24/32/48/256; it is our coverage target, not a claim that Win32 requires every entry. Draft Fluent-style options for the maintainer before producing final resources. Slint can display an icon; generating and embedding the Windows resources remains build work.

## Identity and notifications

Choose one fixed, version-independent AUMID and use it consistently for the process and installed shortcut. Explicit AppUserModelIDs tie together taskbar grouping, pinning and Jump Lists; Microsoft says IDs should be set during initial startup, before presenting UI. [Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids). Slint does not replace this Shell registration: use `windows` and installer metadata.

For the classic unpackaged desktop toast route selected here, install a Start menu shortcut with the AUMID; Microsoft recommends doing this in “the Windows Installer”. [Enable desktop toasts](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid). Set its `System.AppUserModel.ToastActivatorCLSID` and register the matching COM activator so clicking a toast can start the app and dispatch its action. [COM toast activation example](https://learn.microsoft.com/en-us/windows/apps/develop/cpp-winrt/author-coclasses), [ToastActivatorCLSID property](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-toastactivatorclsid).

This is a route-specific requirement, not a universal rule for every unpackaged notification library. The newer Windows App SDK can handle registration automatically. [App notification registration](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-dotnet?pivots=winforms). For the Rust implementation, prove activation both while running in the tray and after exit; Slint alone is not the notification transport.

## Look and window behaviour

- **Mica:** use `DwmSetWindowAttribute` with `DWMWA_SYSTEMBACKDROP_TYPE` and `DWMSBT_MAINWINDOW` through `windows`. Microsoft identifies that value as “Mica in its default variant”. The documented API starts at Windows 11 build 22621, so detect support and use a solid fallback on earlier Windows 11 builds. Do not silently raise the chosen Windows 11 baseline. [DWM backdrop types](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type), [window attributes](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute).
- **Accent and themes:** follow the user's accent colour and light/dark preference. Microsoft uses accent sparingly for emphasis and control state. [Windows colour guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/color). Slint's `fluent` style follows system light/dark settings; fixed `fluent-light`/`fluent-dark` variants override them. [Widget styles](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/). Its [Palette](https://docs.slint.dev/latest/docs/slint/reference/std-widgets/globals/palette/) exposes style brushes, but that page does not promise Windows accent tracking. Verify it; bridge system colours through `windows` if needed, using [UISettings.GetColorValue](https://learn.microsoft.com/en-us/uwp/api/windows.ui.viewmanagement.uisettings.getcolorvalue?view=winrt-26100).
- **Typography:** use Segoe UI Variable. Start from the Windows type ramp in effective pixels: caption 12/16, body 14/20, body large 18/24, subtitle 20/28 and title 28/36 (size/line height). Microsoft recommends “one font throughout” the UI. Set and verify font selection in Slint; do not assume Fluent styling chooses the correct font or optical sizing. [Windows typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography).
- **Contrast themes:** these are separate from dark mode. Replace brand/accent colours with system semantic foreground/background pairs in forced-colour conditions; test customised contrast colours too. [Contrast themes](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes). Use `windows` to detect high contrast with `SPI_GETHIGHCONTRAST`; the [SystemParametersInfoW contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow) documents that query. Automatic Slint Fluent contrast-theme coverage was not established; treat it as a gap to prove.
- **Text scaling:** Windows supports text-only scaling from 100% to 225%, separate from display DPI. Microsoft tells custom frameworks to handle `TextScaleFactorChanged`; controls must resize and reflow. Bridge the setting through `windows` where Slint does not already honour it, and test both scales together. [Text scaling](https://learn.microsoft.com/en-us/windows/apps/develop/input/text-scaling).
- **Snap and minimum size:** keep system caption buttons. For custom buttons, `WM_NCHITTEST` must return `HTMAXBUTTON` over maximize/restore. Microsoft's maximum recommended minimum width is 500 effective pixels, preferably 330 or less. Our approximately 500 epx target trades away some smaller Snap zones; it is not 500 physical pixels at every DPI. [Snap layout guidance](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-snap-layout-menu).

Slint's [winit backend](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/) provides the platform window. [Window::window_handle](https://docs.slint.dev/latest/docs/rust/slint/struct.Window) requires `raw-window-handle-06`; the handle may become available only after showing the window and iterating the event loop. Use that handle for the HWND integration above. Prove that the selected renderer and window background actually expose Mica; merely setting a DWM attribute does not constitute visual proof.

## Accessibility

Windows [UI Automation](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-uiautomationoverview) lets screen readers inspect and act on controls. Slint documents [AccessKit integration](https://slint.dev/blog/slint-1.1-released) and its default-enabled [`accessibility` feature](https://docs.slint.dev/latest/docs/rust/slint/docs/cargo_features/). This is the chosen route to Narrator, rather than adding a separate accessibility process.

Custom components still need roles, labels, state and action callbacks. Slint requires `accessible-role` before other accessibility properties can be set. [Accessibility properties](https://docs.slint.dev/latest/docs/slint/reference/common/#accessibility-properties). Prove keyboard navigation, focus order, announced labels/states and actions for Home and About in M1, then each later screen. Do not claim complete Narrator coverage from a feature flag. One documented control gap is the `tree` accessibility role, marked “Not provided yet” in the same reference. No comprehensive current Windows gap list was found in the consulted Slint docs. The [web backend](https://docs.slint.dev/latest/docs/slint/guide/platforms/web/) explicitly lacks screen-reader accessibility; that documented limit does not describe the selected desktop backend.

## Startup and console

The per-user Run key registers a command to run at sign-in. It is a registration mechanism, not an effective-state query, and startup can be delayed. [Run and RunOnce](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys). The project requires an on-by-default Start at sign-in checkbox that reflects Windows Startup Apps changes and never silently reverses a user's disable choice.

**Not found:** a supported Microsoft Learn contract for decoding `Explorer\StartupApproved` or reading the effective Startup Apps state of an arbitrary unpackaged Run entry. Do not document guessed byte values or treat value existence as proof of enabled startup. Resolving that state query and proving it against the Windows toggle is groundwork for the `windows` implementation; Slint supplies the checkbox only.

Select the Windows subsystem for release builds to avoid a console window. Slint gives `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` as the Rust form. [Slint on Windows](https://docs.slint.dev/latest/docs/slint/guide/platforms/desktop/windows/general/). Microsoft distinguishes console applications from applications that do not require a console in its [subsystem documentation](https://learn.microsoft.com/en-us/cpp/build/reference/subsystem-specify-subsystem). Preserve usable file logging and error reporting when standard output is unavailable.

## Signing readiness

Authenticode protects integrity and identifies the certificate signer. Use SHA-256 for the file digest and an RFC 3161 timestamp (`/fd SHA256`, `/tr`, `/td SHA256`), then verify with SignTool's Authenticode policy (`verify /pa`). Microsoft recommends “SHA256”. [SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool). Proposed release order, inferred from signing final file bytes: sign owned EXEs/DLLs first, package them, then sign the outer MSI; verify both layers and make no post-sign edits. Keep this distinct from the minisign update signature required by ADR 0005.

Embed numeric `FILEVERSION` and `PRODUCTVERSION`, plus `StringFileInfo` values for `CompanyName`, `FileDescription`, `FileVersion`, `InternalName`, `OriginalFilename`, `ProductName`, `ProductVersion` and copyright. Microsoft defines the version resource as holding “version number” and “original filename”. [VERSIONINFO](https://learn.microsoft.com/en-us/windows/win32/menurc/versioninfo-resource). Use the project name and a consistent release version across owned binaries and installer metadata. Resource generation is build work, outside Slint's UI.

### SignPath Foundation

The [Foundation terms](https://signpath.org/terms) require OSI-approved licensing without commercial dual-licensing, no proprietary components except System Libraries, an actively maintained released project, and verifiable build origin. They require MFA, defined contributor/reviewer/approver roles and manual release signing approval. Product names must match the project and product versions must agree within a build. A Code signing policy must appear or be linked on home and release/download pages, with signing attribution, roles and privacy policy. Upstream unsigned binaries may be included in signed packages such as MSI, but must not be signed as our own.

Our choice: GitHub-hosted builds only, per-user WiX MSI, opt-out update checks and a PRIVACY page shown during installation. These implement the origin/privacy requirements; the consulted Foundation pages did not establish a universal GitHub-hosted-only rule or an NSIS support matrix. MSI is explicitly mentioned; do not promise NSIS eligibility from generic Authenticode support. The policy must describe signing truthfully while releases remain unsigned. GPL-3.0 distribution with Apache-2.0 own files follows [Slint's published licensing option](https://slint.dev/community); acceptance still needs Foundation review.

### Azure Artifact Signing

Public Trust individual applicants must be in the United States or Canada, pass identity validation, and use an Individual billing account whose details match their government ID. [Setup prerequisites](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart). The service requires a paid Azure subscription; free/trial/sponsored subscriptions are unsupported. [FAQ](https://learn.microsoft.com/en-us/azure/artifact-signing/faq).

Basic is $9.99 per account per month for 5,000 signatures; excess signatures are $0.005 each. Treat these as research-date prices. [Pricing tiers](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-change-sku). Microsoft describes daily issuance of certificates valid for about three days, so timestamping matters. [Signing overview](https://learn.microsoft.com/windows/msix/package/signing-package-overview). The documented GitHub integration is `azure/trusted-signing-action` (the older service name persists in tooling). [GitHub Actions example](https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide#cicd-github-actions). This is an alternative signing service, not a change to the chosen unsigned-first release plan or MSI format.

## Gaps in today's Companion

Verified in the checkout used for this note; these are source observations, not Windows runtime measurements.

- [app/crates/td-companion/src/tray/windows.rs](../../app/crates/td-companion/src/tray/windows.rs) loads `IDI_APPLICATION`, sets `uID = 1`, supplies no GUID, and opens the same menu for `WM_LBUTTONUP` and `WM_RBUTTONUP`. It does not set version 4 or handle `TaskbarCreated`.
- [app/crates/td-companion/src/main.rs](../../app/crates/td-companion/src/main.rs) has no `windows_subsystem` attribute, so it does not request suppression of the console through that mechanism.
- [app/crates/td-companion/icon.ico](../../app/crates/td-companion/icon.ico) (then `app/electron/installer/icon.ico`) contains 16/24/32/48/64/256 entries. Measured by reading the ICO directory, treating a zero width/height byte as 256; it lacks the other sizes in the coverage target.
- [app/crates/td-companion/src/startup.rs](../../app/crates/td-companion/src/startup.rs) returns true when the Run value exists and never consults `StartupApproved`; this is registration state only.
- [app/crates/td-companion/src/process.rs](../../app/crates/td-companion/src/process.rs) calls `startup.set(true)` on packaged startup when the `startup-initialized` marker is absent, then writes the marker. The native setup must make the default visible and respect later user choices.

## Implementation checklist

Each identifier is stable so later issues can link to `#implementation-checklist` and name the item. These are acceptance requirements, not claims that the work is complete; platform sources and limits are above, project choices are in ADR 0011.

- [ ] W01: Run Slint and the Companion in one process with a live tray after the window closes.
- [ ] W02: Preserve a tray GUID and executable path across upgrades; preserve the user's placement/visibility preference.
- [ ] W03: Re-add the tray icon and set version 4 after `TaskbarCreated`.
- [ ] W04: Open/restore on left-click and keyboard activation; show the native menu on right-click/context activation and restore focus on dismissal.
- [ ] W05: Load the tray icon at the current DPI and ship all fourteen listed ICO sizes after the maintainer selects an icon.
- [ ] W06: Fix the AUMID across process, shortcut, taskbar grouping, pinning and any Jump Lists.
- [ ] W07: Install AUMID/ToastActivatorCLSID shortcut and COM registration; prove toast activation while running and after exit.
- [ ] W08: Show Mica on supported Windows 11 builds and a solid fallback where unavailable.
- [ ] W09: Follow live accent and light/dark changes in standard and custom controls.
- [ ] W10: Use Segoe UI Variable and the Windows type ramp.
- [ ] W11: Honour contrast themes and customised system colours throughout the app.
- [ ] W12: Reflow at 100–225% text scale independently of display DPI without hiding actions.
- [ ] W13: Preserve maximize-button Snap layouts and usability at approximately 500 epx minimum width.
- [ ] W14: Keep AccessKit enabled and prove Narrator roles, labels, states, actions, keyboard navigation and focus order.
- [ ] W15: Show Start at sign-in on by default; resolve and read effective Windows startup state, including an external disable, without overriding it.
- [ ] W16: Explain close-to-tray once and offer an explicit Quit action.
- [ ] W17: Suppress the release console and retain file logging and visible errors.
- [ ] W18: Ship per-user WiX MSI, block Windows 10 clearly and retain 1.x on Releases.
- [ ] W19: Embed consistent VERSIONINFO, product name and release version before packaging.
- [ ] W20: Publish a truthful code signing policy and PRIVACY page; allow opting out of update checks during installation and in Settings.
- [ ] W21: Build release artifacts only on GitHub-hosted runners and retain verifiable source/build origin for future signing.
- [ ] W22: Prepare signing inside-out with SHA-256, RFC 3161 timestamps and verification of owned binaries and MSI.
- [ ] W23: Keep the minisign key and prove Windows 10 exclusion. The 1.x updater bridge and data migration were dropped on 2026-11-13 (ADR 0013); the maintainer replaces 1.x by hand.
- [ ] W24: Capture Windows screenshots and RAM on GitHub-hosted Windows runners with OS/build, workload, build mode and memory metric recorded.
- [ ] W25: Complete M0–M5 before deleting Electron, React and the browser/local web API at M6; update GLOSSARY.md at that cutover. (ADR 0012 moved Electron and React to `release/1.x` early and updated GLOSSARY.md; the local web API still goes at M6.)
