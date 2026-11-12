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
`td_companion::editor::Editor`. The frozen React editor and local web API keep
their current interfaces until cutover (ADR 0011).

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
keeps one `Editor` owner per Device under the Companion data directory. Selection,
window state, dialogs and Device I/O belong to adapters. The lifecycle is:

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

The M3 UI must connect this lifecycle to the Device lane and schedule persistence.
Further transformations (Layout/Page names, grid size and full-screen settings),
Preset fallback policy, asynchronous GIF preparation and migration of browser
local-storage drafts remain follow-ups. Firmware enforcement and schema format
fixtures remain independent.
