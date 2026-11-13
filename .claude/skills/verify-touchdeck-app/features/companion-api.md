# Companion feature coverage

All routes below are relative to `/api/v1`. Start an isolated mock, Doctor it, then use `$TA api GET /devices` and `$TA events --seconds 2` for state and event evidence. For writes, read the endpoint method and body in the cited Rust file; Jobs require polling `/jobs/{id}` to terminal success. API success alone does not prove UI behavior.

## actions

Source: `app/crates/td-companion/src/features/actions.rs`.

- `/approvals`
- `/approvals/{id}/show`
- `/actions/log`
- `/actions/allowed`
- `/actions/allowed/{hash}`

## assets

Source: `app/crates/td-companion/src/features/assets.rs`.

- `/assets/builtin`
- `/assets/builtin/suggest`
- `/assets/{asset}`
- `/assets`
- `/devices/{device}/assets/prune`
- `/assets/catalog/search`
- `/devices/{device}/assets`

## devices

Source: `app/crates/td-companion/src/features/devices.rs`.

- `/devices`
- `/devices/{device}`
- `/devices/{device}/connect`
- `/devices/{device}/disconnect`
- `/devices/{device}`
- `/devices/{device}/restart`
- `/devices/{device}/settings`
- `/devices/{device}/settings`
- `/new-boards`
- `/ports`
- `/ports/rescan`
- `/devices/{device}/console`

## integrations

Source: `app/crates/td-companion/src/features/integrations.rs`.

- `/integrations`
- `/integrations/{integration}`
- `/integrations/{integration}/config`
- `/integrations/{integration}/secrets/{field}`
- `/integrations/{integration}/secrets/{field}`
- `/integrations/{integration}/refresh`

## jobs

Source: `app/crates/td-companion/src/features/jobs.rs`.

- `/jobs`
- `/jobs/{job}`
- `/jobs/{job}`

## layouts

Source: `app/crates/td-companion/src/features/layouts.rs`.

- `/devices/{device}/layouts/{orientation}`
- `/devices/{device}/layouts/{orientation}`

## mod

Source: `app/crates/td-companion/src/features/mod.rs`.


## system

Source: `app/crates/td-companion/src/features/system.rs`.

- `/system`
- `/system/shutdown`
- `/system/lock`
- `/system/lock/{lock}`

## test_hooks

Source: `app/crates/td-companion/src/features/test_hooks.rs`.

- `/test/devices/{device}/console` (see [device console](device-console.md); real Devices only with `--test-real-device-console`)
- `/test/devices/{device}/mock`
- `/test/devices/{device}/press`
- `/test/devices/{device}/sets`
- `/test/devices/{device}/record`
- `/test/devices/{device}/emit`
- `/test/input-log`
- `/test/ports`
- `/test/clock`
- `/test/approvals/script`
- `/test/firmware/script`
- `/test-hooks/media`
- `/test-hooks/volume`
- `/test-hooks/volume`

## uploads

Source: `app/crates/td-companion/src/features/uploads.rs`.

- `/uploads`

## Authentication and events

`src/api/auth.rs` covers ping, runtime proof, window sessions, browser pairing, revocation and pairing reset. `src/api/socket.rs` covers hello/sync, state fanout, subscriptions and session closure. `pairing.companion` drives real browser pairing; `companion` drives layout, Device lifecycle, actions, Firmware consent and key-monitor events. API contract, auth boundaries, limits, Jobs, Assets, uploads, Firmware, photo frames and persistence have Rust tests in `app/crates/td-companion/tests`; use `cargo test -p td-companion` for focused source work. Test hooks are compiled only in test builds and are not product API guarantees.

## Feature proof boundaries

- Devices, ports, New boards, settings and console: `companion`, `device-panel`, `new-board`, `onboarding`; a real console round trip also needs a later board pass, which can drive it with `$TA start --device-console` and `$TA devcmd`.
- Layouts and Jobs: `layout`, `companion`; poll Jobs and inspect mock stored Layouts.
- Assets and uploads: `icon-picker`, `gif`, `media`, plus Rust Asset/upload tests; mocks cannot prove physical playback.
- Integrations: `integrations`, `integrations.companion`, `custom.companion`, `account-detail.companion`, `this-pc`, `media`, `graph`. Use local fixtures for API credentials.
- Actions, allowed commands and pending native approvals: `inspector`, `companion`; scripted consent proves the protocol, not a human answering the native dialog.
- Firmware jobs and consent: `new-board`, `companion`; real writes wait for the board pass.
- System identity, shutdown and install locks: Doctor, stop, Rust system tests and Electron update-lock tests. `settings` exercises system settings; `updates` exercises fake updater states, not installer execution.
