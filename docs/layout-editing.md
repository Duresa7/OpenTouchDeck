# Native Layout editor model

Three shapes were considered:

- A td-app controller could compose Editor and Device operations, but would put
  persistence, Asset preparation and conflict policy beside UI code.
- A shared Handle-owned editor registry could broadcast snapshots, but adds locks
  and ownership rules without a second editor consumer.
- A per-Device Companion Session can own the drafts and expose commands and
  snapshots to one native adapter.

The third shape is chosen. `Handle::open_editor(device)` opens an
`editor::Session`. Persisted drafts, conditional revisions, Assets and Device
operations belong in td-companion. Slint owns window state, dialogs and rendering.
Keep one Session owner per Device. There is no undo/redo: the 1.x reference editor
had none. No UI is implemented here.

## Native adapter contract

- Render `draft()`, `selection()` and `save_state()`. `Draft::effective_tile()`
  includes rejected label/action values; `rejected()` carries structured rules,
  pointers and `action_field()` for form errors. Refused structural edits leave
  the draft unchanged. Sending selects the first invalid Tile.
- Use `new_layout(Blank | Starter)`, `start`, `import`, `export`, `edit`, `select`
  and `select_orientation`. `add_target`, `add`, `preset_tile` and `place_preset`
  share placement rules. Clear Tile is `Edit::RemoveTile`. Page edits, geometry,
  renderers, input and actions all cross the validated Document interface.
- `load()` preserves a different draft as a conflict; `resolve(KeepMine |
  UseDevice)` adopts the read revision. A missing orientation clears a stale
  revision while keeping the draft. Corrupt storage containers are reported;
  saved Layouts that no longer validate are dropped independently by orientation.
- Drive `changed().await` while the editor is open, then render another snapshot.
  It consumes Companion events, refreshes durable Asset replacements and applies
  GIF results. Replacements also apply when opening, importing, resolving and
  editing, so an old Asset chosen from Recents becomes its replacement. Known
  Device copies stay unchanged. `prune_assets` preserves both drafts' Assets.
- `numeric_streams()` supplies ordered numeric choices with Preset labels and
  units. `bar_appearance()` supplies only a matching Integration/stream Preset.
  Graph Presets fall back to a plain value without graph Firmware; bars and
  now-playing retain their renderer so the UI can show a Firmware requirement.
- `add_gif` and `gif(Choose | Cancel | Retry | Fit)` own the GIF queue, files,
  preparation and refits. Render `gif_work()` and `gif_fit()`. `gif_target()` uses
  Device-space geometry and panel capabilities. Edits and capability events
  refit retained sources; another Device can reuse a source during this Companion
  session. Missing sources ask for the file again. Replaced/cancelled jobs cannot
  write their results into another Tile or resurrect a deleted Tile. Session drop
  cancels its preparation jobs. Uploads are limited to 16 MiB before queuing.
  Native preparation shares normalization, cancellation, limits and admission with
  the existing Asset service. `inspect_gif`, `fitted_text`, `gif_problem` and
  `send_problem` preserve the reference preview and refusal rules without UI code.

## Sending without borrowing the UI

`begin_send(capabilities)` snapshots a push and starts owned work on the current
Tokio runtime. The Session remains editable. Consume `changed()` to render Job
progress through `push_progress()`; await `PendingSend::wait()` on a worker, then
apply its opaque receipt with `finish_send()`. A receipt belongs to one Session
and one active send. Later edits and orientation changes remain intact; only a
matching Device readback marks the sent snapshot known. `push_report()` supplies
counts, transfer measurements and Asset results. `send()` is the convenience
wrapper for callers that do not edit while awaiting the transfer. Device writes
always go through `Handle::layout_operations()`, including the Pushed copy policy.

The existing Editor and Document remain lower layers for deterministic admission
and lifecycle tests. Browser local-storage migration is outside this native model;
its new durable format is separate from 1.x browser storage.

# Rust Layout editing

Three interface shapes were considered:

- Standalone functions over `Layout` are simple to call, but its public fields
  let callers bypass validation between transformations.
- A command-based validated document keeps admission and transformations behind
  one interface, while leaving Device sessions and storage outside `td-core`.
- A Companion service owning both validation and sessions centralizes behavior,
  but couples pure Layout rules to persistence and Device lifecycle decisions.

The command-based validated document was chosen. It gives native UI adapters and
tests the same invariant-preserving seam, without making core rules depend on a
Companion session. A separate Companion editor owns the stateful lifecycle.

The Rust seam for the native editor is `td_core::editing::Document` and
`td_companion::editor::Editor`. The local web API keeps its current
interface until cutover (ADR 0011). The React editor lives on `release/1.x`
(ADR 0012).

The core module admits files and typed Layouts through the existing Layout parser.
Its `Edit` commands produce a new Document atomically. Callers cannot mutate the
Document's Layout. It owns placement validation, span-aware duplicate placement,
Page removal and navigation repair, renderer field cleanup, action/sink changes,
and Asset reference replacement. Renderer changes choose schema 1 or 2 from the
remaining fields. Choosing the Companion sink omits the wire `sink` field.

Compatibility is a separate query: a valid offline draft can still name Bluetooth.
`Capabilities { sinks: None }` means unknown diagnostics, including an offline
Device without cached diagnostics. `Some(&[])` means known lack of Bluetooth
support. Cached diagnostics may be used offline. `Compatibility::Unknown` is
separate from `Unsupported`; Bluetooth consumer usages above 1023 are unsupported
even without diagnostics. This reuses Layout's transfer-time sink validation.

The Companion module holds one Device's two orientation drafts. A native adapter
keeps one `Editor` owner per Device under the Companion data directory. The Session owns native selection and Device I/O; window state and dialogs belong to adapters. The lifecycle is:

1. `Editor::open(data_dir, device_id)` reads the Device's drafts. `start(layout)`
   admits an import or a newly built Layout without losing a known Device revision.
2. `edit(orientation, command)` crosses the same Document interface as tests.
   Structural refusals leave the draft unchanged. Rejected label/action values
   remain available through `effective_tile`, alongside `rejected()` diagnostics.
   Duplicating a Tile copies these effective rejected fields. Structural changes
   recheck diagnostics; a field that becomes admissible joins the valid draft.
3. `persist()` explicitly checkpoints both orientations, rejected fields, known
   revisions and conflicts atomically to `editor-drafts-v1/<device>.json`.
   Read failures are reported without overwriting the stored drafts. This is a new
   Rust format, separate from browser storage, stored Layouts and Pushed copies.
4. `prepare_push(orientation, capabilities)` rejects unresolved fields, conflicts,
   unsupported destinations or unknown capabilities required by this Layout.
   Its immutable `PushPlan` supplies a Layout and optional expected revision.
   With no known revision, the first send is unconditional, matching the current
   editor. The Device adapter must enforce a supplied expected revision.
5. On a conditional-send conflict, read the Device Layout and call
   `receive(layout, revision)`. It preserves a different draft until
   `resolve(orientation, KeepMine | UseDevice)`. Keeping the draft adopts the new
   revision for the next conditional send; taking the Device copy clears rejected
   fields. Receiving an identical clean Layout establishes a known copy directly.
   If the read finds no stored Layout, `receive_missing(orientation)` preserves
   the draft and clears the stale revision for a retry.
6. After sending and reading back, `finish_push(plan, displayed, revision)` verifies
   the displayed Layout before establishing the sent copy. Later edits remain
   unsent. A stale plan cannot overwrite a changed baseline or pending conflict.

`replace_assets` applies a supplied replacement map transactionally to both open
drafts, preserving the last known Device copies. Replacement keys and values use
full `sha256:` wire references. The Asset owner supplies replacement notifications.

The Session connects this lifecycle to the Device lane and checkpoints accepted edits and rejected fields.
Layout/Page names, grid size and full-screen settings are validated Edit commands.
Migration of browser local-storage drafts remains outside this native model. Firmware enforcement and schema format
fixtures remain independent.
