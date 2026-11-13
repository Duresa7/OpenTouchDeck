# Bluetooth keyboard address privacy

On 2026-11-13, review of [PR #390](https://github.com/Duresa7/OpenTouchDeck/pull/390) identified a gap outside its private discovery payloads. [Issue #401](https://github.com/Duresa7/OpenTouchDeck/issues/401) recorded that the Bluetooth keyboard role advertised indefinitely from its fixed, factory-derived public address when no bonded computer or phone was connected. Rotating discovery hints could not prevent tracking through that address.

On 2026-11-13, [PR #390 at OpenTouchDeck commit e4a4559](https://github.com/Duresa7/OpenTouchDeck/blob/e4a455997eb5a662f5f92471db985447e00f8768/schema/protocol/net-v2.md) documented the remaining stable identifier. It did not fix the keyboard role. [#401](https://github.com/Duresa7/OpenTouchDeck/issues/401) tracked the fix to use a resolvable private address while preserving reconnection for bonded computers and phones; [#284](https://github.com/Duresa7/OpenTouchDeck/issues/284) remained open for that work. No completed fix PR was recorded for #401 in this entry.

On 2026-11-13, [#401](https://github.com/Duresa7/OpenTouchDeck/issues/401) also separated the address blocker from follow-up hardening: advertising fields, readable Device name, rotation timing, random-number requirements, replay handling, recognition during pairing mode and MAC-buffer clearing. Its acceptance criteria covered every BLE role, not only discovery service data.

Related: [M4's keyboard controls](m4-design.md), [test crypto alerts](test-crypto-alerts.md), [review triage](review-and-merge.md).
