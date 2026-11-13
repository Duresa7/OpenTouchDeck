# Layout operations design (#343)

Considered interfaces:

- Separate prepare and commit calls: callers would own the revision race and Job ordering.
- A command enum: one entry point, but dispatch adds no useful hiding for three operations.
- `Handle::layout_operations()` with `read`, `last_seen`, and `push`: chosen. Native callers and tests cross the same small interface. `push` accepts Layout bytes and an optional revision condition; the web adapter alone parses HTTP headers.

This deep module hides both revision checks, Asset pinning and transfer, Action sink and animation validation, Job sequencing, local-copy persistence and the authority update. Existing ordering and errors are preserved. The operations borrow no HTTP request or adapter state. The module owns its reply and error types; the adapter re-exports their existing public paths.

## Tests moved across the interface

The shared push helper in `tests/layouts.rs` now calls the module through the running Companion handle. This moves the push operation exercised by these tests:

- `two_devices_keep_their_own_layouts`: two devices keep their own layouts.
- `an_offline_device_refuses_a_layout_read_even_with_a_copy_on_this_pc`: an offline device refuses a layout read even with a copy on this pc.
- `an_offline_device_shows_the_layout_this_pc_last_saw`: an offline device shows the layout this pc last saw.
- `revisions_orientation_and_validation_are_checked`: revisions orientation and validation are checked.
- `a_second_push_gets_the_busy_job`: a second push gets the busy job.
- `a_cancelled_push_leaves_the_device_alone`: a cancelled push leaves the device alone.
- `a_push_sends_the_assets_the_device_lacks`: a push sends the assets the device lacks.
- `control_layout_get_sync_and_updates_agree_after_push_and_reconnect`: control layout get sync and updates agree after push and reconnect.
- `a_push_publishes_only_checking_then_matches`: a push publishes only checking then matches.
- `control_layout_reports_unsupported_firmware_and_recovers_after_an_abandoned_read`: control layout reports unsupported firmware and recovers after an abandoned read.

HTTP tests retain ETag, If-Match, validation, status and error translation coverage. No behavior bug was found or changed.

The pushes in `tests/assets.rs` also cross the module interface:

- `prune_keeps_what_stored_layouts_and_the_caller_use`: transferred Assets referenced by a stored Layout survive pruning.
- `a_running_layout_push_pins_its_assets_against_a_sweep`: a delayed schema 2 transfer keeps its Assets pinned while a cache sweep runs.

`read` and `last_seen` assertions in the Layout tests also use the module. The busy-Job assertion checks the typed module error directly.
