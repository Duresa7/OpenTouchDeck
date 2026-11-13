# Device console through the Companion

`test.console` (`POST /api/v1/test/devices/{device}/console`, body `{line, wait_ms?}`) sends one Firmware console line through the Companion's own link and returns `{reply, lines}`. It exists only in `test-hooks` builds. It answers for mock Devices, and for real Devices only when the Companion was started with `--test-real-device-console` (`$TA start --device-console`). Without that argument a real Device gets `route.not_found`, and every other test route stays `route.not_found` on real ports either way. Source: `app/crates/td-companion/src/features/test_hooks.rs`; tests: `test_console_*` and `the_release_router_has_no_test_console` in `app/crates/td-companion/tests/devices.rs`.

The route subscribes to the Device's console lines before it writes, then returns at the first `@td` reply whose `cmd` matches the line's first word (or its first 63 characters), or at `wait_ms` (default 3000, at most 15000; 0 returns what arrived during the send). `reply` is null when none came in time. `lines` keeps log lines that arrived meanwhile, at most 256.

## Board pass with the Companion up

1. Take the hardware lock, then `$TA build` and `$TA start --device-console`. `$TA api GET /devices` names the Device.
2. `$TA devcmd "ui state"` and the other commands below. The reply JSON is the Firmware's, as `$TD serial cmd` prints it.
3. `$TA stop` before any `$TD serial screenshot`, `capture`, `keys` dump, `store put|get`, flash or reset capture. Release the lock when done.

Single-reply commands that work: `touch tap|swipe|log|stats`, `ui state|goto|tile|hold`, `keys tap|consumer|mod|clear|state|log|fuzz`, `keys record start|stop`, `power state|idle-timeout`, `perf fps <s>` with `--wait-ms` of `(s+3)*1000` up to 12 s, `perf mem`, `backlight on|off`, `layout state|reload|use`, `link state`, `version`, `heap`, `uptime`, `store stat|ls|rm`, `settings`. Not usable as commands: `screenshot`, `keys record dump` and `store get` stream many lines after their first reply (`screenshot` is the console flood, see [board pass tools](board-pass-tools.md)), `store put` needs continuation lines, and `reboot` drops the link.

Steps from the board pass that this unblocks, with the Companion and a real player up: Tile taps and drags on Volume and Media Tiles (#97/#98 Step C steps 2 to 5, Step H steps 2, 3, 5, 7, 10, 14, #97 Step E step 3), tap latency while uploading (#93 step 5, #94 step 5, #94 Step E step 1, #98 Step F step 3), the photo-frame wake taps with `ui state` and `keys record` (#102 steps 2 and 3), the Starter Layout tap after New board setup (#85 step 3), and touches after Send to Device (#201 step 1). Injected touches prove LVGL logic, not the glass; native consent, power, photographs and the look of the panel still need a person.
