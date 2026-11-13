# M4 Integrations and Device panel design

On 2026-11-13, the [design comment on #356](https://github.com/Duresa7/OpenTouchDeck/issues/356#issuecomment-6845285016) narrowed M4 around the M2 work in #387. Paired apps and bundled Firmware updates belonged to M2; M4 added Firmware from a file, the build switch and Bluetooth keyboard management. This was an implementation plan, not a completion report.

On 2026-11-13, that [design comment](https://github.com/Duresa7/OpenTouchDeck/issues/356#issuecomment-6845285016) chose Companion-owned `IntegrationsSession`, `IconPacksSession`, `BrandfetchSession` and `BluetoothSession` interfaces. Slint adapters would map their views to properties, while headless tests exercised the rules. `FirmwareTask` would gain a file source. The five registry Integrations would share one schema-driven detail page; Icon packs and Brandfetch would have separate pages.

## Scope decisions

On 2026-11-13, the [M4 design comment](https://github.com/Duresa7/OpenTouchDeck/issues/356#issuecomment-6845285016) set these boundaries:

- Removing a registry Integration meant switching it off and forgetting its secret. One Save would handle fields and secrets together. Secret fields would show Saved, Replace and Forget, never the secret value.
- Icon pack and Brandfetch switches would move from browser storage to `AppSettings`. Brandfetch search and pack thumbnails would wait for M3's icon picker and SVG rendering.
- Bluetooth keyboard controls would appear only when the Companion carried a radio bundle or the Device ran one. Wi-Fi setup and Wi-Fi Pairing were outside M4.
- Repair and the Firmware console would wait for Troubleshooting work. Command approval and Firmware consent would keep the native prompts required by ADR 0006; CI would use the scripted prompter.
- The custom Integration would keep its raw JSON field and pending-approval message.

## Ticket sequence

On 2026-11-13, [#393 through #400](https://github.com/Duresa7/OpenTouchDeck/issues/356#issuecomment-6845285016) divided that plan as follows:

| Ticket | Work | Dependencies recorded on 2026-11-13 |
| --- | --- | --- |
| [#393](https://github.com/Duresa7/OpenTouchDeck/issues/393) | Split M2 screen and test files. | #387. |
| [#394](https://github.com/Duresa7/OpenTouchDeck/issues/394) | Integration session and its headless tests. | No prerequisite in the ticket. |
| [#395](https://github.com/Duresa7/OpenTouchDeck/issues/395) | Integration list and detail screens. | #393, #394 and #387. |
| [#396](https://github.com/Duresa7/OpenTouchDeck/issues/396) | Icon packs session, page and stored switch. | #395; coordinate the interface with M3. |
| [#397](https://github.com/Duresa7/OpenTouchDeck/issues/397) | Brandfetch session, page and stored switch. | #395; coordinate the interface with M3. |
| [#398](https://github.com/Duresa7/OpenTouchDeck/issues/398) | Firmware file source and build switch. | #393 and #387; sequence with #399. |
| [#399](https://github.com/Duresa7/OpenTouchDeck/issues/399) | Bluetooth keyboard session and controls. | #393 and #387; sequence with #398. |
| [#400](https://github.com/Duresa7/OpenTouchDeck/issues/400) | Windows screenshot evidence. | #395 through #399. |

Related: [2.0 plan](native-app-plan.md), [overlapping branches](parallel-edit-conflicts.md), [Bluetooth keyboard privacy](ble-keyboard-privacy.md).
