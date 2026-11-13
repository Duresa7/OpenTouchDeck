# Native editor session design (M3 foundation)

Three shapes were considered for the native UI:

- A td-app controller could compose the existing Editor and Device operations,
  but would put persistence, transfer and conflict policy beside UI code.
- A Handle-owned shared editor registry could broadcast snapshots, but adds
  locking and ownership rules before there is a second native editor consumer.
- A Companion Session opened through Handle can own one Device editor and expose
  drafts and selection directly to a single native adapter.

The third shape is chosen: `Handle::open_editor(device)` returns an
`editor::Session`. Its small interface starts/imports a Layout, selects an
orientation/Page/Tile, applies validated commands, loads, sends and resolves a
conflict. The UI renders `draft()` and `selection()` after each operation. There
was no undo/redo in the reference editor. The existing pure Document and persisted
Editor remain the lower layers. The Session hides conditional push, Job completion,
readback and checkpoints. This belongs in td-companion because persisted drafts
and Device revision decisions must have the same owner regardless of UI toolkit.
Keep a single Session owner for each Device. Async calls currently borrow it
exclusively; UI work should run these away from the Slint event loop.

This is a foundation, not the complete M3 port. Still required: Tile-bound GIF
preparation (queue, cancel/retry, refit after geometry/capability changes, source
retention and stale-result suppression); automatic Asset replacement observation;
Add Tile defaults/target/new-Page placement and Preset placement; numeric stream
picker queries; every remaining TypeScript test mapped through the public Session
against the mock Device. The UI itself remains a separate task.

`placed_appearance` owns Preset renderer fallback: graphs require numeric streams
and graph Firmware, bars and now-playing retain their renderer even when Firmware
needs updating, invalid declarations fall back to a plain value. Placement must
still validate the Preset action through Document.

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

The M3 UI must connect this lifecycle to the Device lane and schedule persistence.
Layout/Page names, grid size and full-screen settings are validated Edit commands.
Asynchronous GIF preparation and migration of browser local-storage drafts remain follow-ups. Firmware enforcement and schema format
fixtures remain independent.
