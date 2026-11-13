# App verification map

Run commands from [SKILL.md](../SKILL.md). Fake specs prove client behavior with demo data. Companion specs use the actual API and mock Devices, with fresh data. Read [Companion API coverage](companion-api.md) for feature routes and proof boundaries. Read [device console](device-console.md) to send Firmware console commands through a running Companion, and [board pass tools](board-pass-tools.md) for the Companion log, a newer Firmware bundle and a console flood.

| Current React spec | Observable checks | Mode |
| --- | --- | --- |
| `account-detail.companion.spec.ts` | [account-detail.companion](account-detail.companion.md) | Companion |
| `break-ui.spec.ts` | [break-ui](break-ui.md) | fake |
| `companion.spec.ts` | [companion](companion.md) | Companion |
| `custom.companion.spec.ts` | [custom.companion](custom.companion.md) | Companion |
| `device-panel.spec.ts` | [device-panel](device-panel.md) | fake |
| `gif.spec.ts` | [gif](gif.md) | fake |
| `graph.spec.ts` | [graph](graph.md) | fake |
| `home.spec.ts` | [home](home.md) | fake |
| `icon-picker-bench.spec.ts` | [icon-picker-bench](icon-picker-bench.md) | fake |
| `icon-picker-shots.spec.ts` | [icon-picker-shots](icon-picker-shots.md) | fake |
| `icon-picker.spec.ts` | [icon-picker](icon-picker.md) | fake |
| `inspector-shots.spec.ts` | [inspector-shots](inspector-shots.md) | fake |
| `inspector.spec.ts` | [inspector](inspector.md) | fake |
| `integrations-shots.spec.ts` | [integrations-shots](integrations-shots.md) | fake |
| `integrations.companion.spec.ts` | [integrations.companion](integrations.companion.md) | Companion |
| `integrations.spec.ts` | [integrations](integrations.md) | fake |
| `layout-shots.spec.ts` | [layout-shots](layout-shots.md) | fake |
| `layout.spec.ts` | [layout](layout.md) | fake |
| `media.spec.ts` | [media](media.md) | fake |
| `motion-shots.spec.ts` | [motion-shots](motion-shots.md) | fake |
| `motion.spec.ts` | [motion](motion.md) | fake |
| `new-board.spec.ts` | [new-board](new-board.md) | fake |
| `onboarding-shots.spec.ts` | [onboarding-shots](onboarding-shots.md) | fake |
| `onboarding.spec.ts` | [onboarding](onboarding.md) | fake |
| `pairing.companion.spec.ts` | [pairing.companion](pairing.companion.md) | Companion |
| `screens.spec.ts` | [screens](screens.md) | fake |
| `settings-shots.spec.ts` | [settings-shots](settings-shots.md) | fake |
| `settings.spec.ts` | [settings](settings.md) | fake |
| `shell.spec.ts` | [shell](shell.md) | fake |
| `stacking.spec.ts` | [stacking](stacking.md) | fake |
| `this-pc.spec.ts` | [this-pc](this-pc.md) | fake |
| `updates.spec.ts` | [updates](updates.md) | fake |

Electron tests live in `app/electron/tests` and its Playwright config; run its own e2e script for shell session trust, navigation guards, window lifecycle and updater behavior. Native frame and physical feel need separate observation.
