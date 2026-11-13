# The 2.0 native app plan

On 2026-11-12, [spec #359](https://github.com/Duresa7/OpenTouchDeck/issues/359) and [ADR 0011 at OpenTouchDeck commit 7616d5a](https://github.com/Duresa7/OpenTouchDeck/blob/7616d5acbee370794a4fd85b6de729712fe03577/docs/adr/0011-native-windows-app-in-rust-and-slint.md) chose one Rust program containing the Companion and a Slint UI. The plan put Windows 11 first, Linux next and macOS later. Memory use, native Windows behaviour and one implementation language motivated the decision; it did not establish a benchmark ranking of UI toolkits.

On 2026-11-12, the original ADR 0011 plan kept Electron and React frozen on main until M6. That timing was superseded the same day by [ADR 0012 at OpenTouchDeck commit 7616d5a](https://github.com/Duresa7/OpenTouchDeck/blob/7616d5acbee370794a4fd85b6de729712fe03577/docs/adr/0012-electron-and-react-leave-main-before-cutover.md). Electron and React left main for `release/1.x`; the Companion's local JSON API remained until cutover. This also superseded the old deletion timing still written in #359 and #358.

## Milestone scope

On 2026-11-13, the milestone issues defined the following work. These are scopes, not a claim that the work had shipped.

| Milestone | Source | Scope recorded on 2026-11-13 |
| --- | --- | --- |
| M0 | [#352](https://github.com/Duresa7/OpenTouchDeck/issues/352) | Architecture, hooks, native CI, toolchain and release groundwork. |
| M1 | [#353](https://github.com/Duresa7/OpenTouchDeck/issues/353) | Tray, window, Home, About, Windows appearance, accessibility and evidence. |
| M2 | [#354](https://github.com/Duresa7/OpenTouchDeck/issues/354) | First-run setup, Device and PC settings, update preference and USB Pairings. |
| M3 | [#355](https://github.com/Duresa7/OpenTouchDeck/issues/355) | The native Layout editor, including drafts, rejected edits, Assets and the Pushed copy check. |
| M4 | [#356](https://github.com/Duresa7/OpenTouchDeck/issues/356) | Integrations and the remaining Device panel work, refined in the [M4 design note](m4-design.md). |
| M5 | [#357](https://github.com/Duresa7/OpenTouchDeck/issues/357) | MSI, signed-update verification, release pipeline and the 1.x migration bridge. |
| M6 | [#358](https://github.com/Duresa7/OpenTouchDeck/issues/358) | Remaining API removal, documentation cutover and the 2.0 release, subject to ADR 0012's earlier removals. |

On 2026-11-13, [PR #388](https://github.com/Duresa7/OpenTouchDeck/pull/388) closed M1, issue #353. At [OpenTouchDeck commit a57ed4a](https://github.com/Duresa7/OpenTouchDeck/commit/a57ed4a88b4ec1ea89ea6e9704532a7c855e8a1a), the evidence workflow covered screenshots, accessibility trees, tray behaviour and memory measurements. Its report explicitly limited what the runner proved: software rendering did not prove GPU memory use or wallpaper-tinted Mica, and an accessibility tree did not prove Narrator speech or a real sign-in.

Related: [review and merge gates](review-and-merge.md), [TypeScript port outcomes](typescript-port-outcomes.md).
