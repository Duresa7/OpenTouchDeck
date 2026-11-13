# Device settings and Pairing operations

Native screens and behavioural tests use `Handle::device_operations()` in
`td-companion::devices`. The local web handlers, kept on `main` only as the integration
tests' harness, are adapters over the same interface. The existing wire names stay
unchanged (ADR 0003); the generated TypeScript contract left `main` with the React UI
(ADR 0012).

The interface adds module depth above DeviceManager: it coordinates input validation,
capabilities, actor execution, Layout authority, accepted-name publication and PC photo
folder persistence. DeviceManager continues to own discovery, links and actors. Name and
Wi-Fi input rules remain with their `td-link` owners.

## Settings

`settings_get(id)` reads settings and updates photo-frame state. `settings_patch(id, patch)`
serializes a Device's reads and changes through the final PC folder save. Validation and
initial-read failures return an error before any changes are attempted. After that read,
the operation returns a `SettingsOutcome` with:

- `settings`: the values acknowledged by the Device, including any accepted name and PC folder.
- `applied`: fields acknowledged before a later failure, in execution order.
- `pending`: requested fields that were not acknowledged. A photo-frame change and its PC folder
  save are separate fields because the Device can accept one while the PC save fails.
- `restart_required`: preserved even when a later step fails.
- `error`: the failure that stopped the operation, if any.

A failed or unanswered write does not imply Device rollback. Use the acknowledged values
to roll back a form draft, retain pending fields for retry, and reread after an unknown
outcome. If photo-frame settings were not reached before a failure, their value is absent;
that absence in a failed outcome does not establish lack of Firmware support. Orientation
writes retain the Layout authority invalidation barrier even when the answer is lost or a
later step fails. A failed PC folder save keeps the previously saved folder in memory.

The web adapter returns its existing settings response on success and its existing error
on failure. The richer partial result is available directly to native screens.

## Pairings

Use `pairings_get`, `pairings_forget` and `pairings_forget_all`. Subscribe with
`observe_pairings()` before reading `pairing_snapshot(id)`. This watch stream coalesces
updates: it describes the current state, not a log of every intermediate phase. Snapshots
contain progress, the last Device list, pending removals, and this Companion's local Pairing.
A link transition marks the cached list stale.

USB reconnect automatically refreshes a previously read Device, even when no removal
remains pending. Each refresh retries outstanding removals before listing. A storage
refusal keeps its pending mark while still allowing a fresh list. Removing this Companion
also removes its local key; if that deletion fails, the mark remains for the next retry.
Consumers do not need to invalidate a second Pairing store after the operation. Pending
marks last for this Companion process, as before.

## Follow-ups for #346 and M2

- Move the legacy TypeScript form's optimistic draft ordering, throttling, confirmation and
  restart timers into the native settings coordinator. The new operation already supplies
  acknowledged values, pending fields and restart results, but does not own a live form draft.
- Move the web store's immediate reread after an unanswered removal behind the Rust interface;
  this step owns reconciliation on subsequent reads and USB reconnect.
- Replace the legacy TypeScript name character table when the frozen web UI is removed; the
  Rust operation already validates every submitted name through `td-link::identity`.
- Add native setup and Settings screens in #354; this module does not edit the M1 UI lane.
